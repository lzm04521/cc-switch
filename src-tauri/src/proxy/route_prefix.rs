//! 会话粘性绑定层（fork 定制，doc/20261009-设计文档-会话路由改造为聚合模式）。
//!
//! 旧「route_enabled 分组 + `G.<key>:<model>` 寻址 + /v1/models 分组条目」体系已彻底删除；
//! 模型 id 的聚合寻址（可配置前缀、短形式、全 id）统一在 [`crate::mode::stack`]。
//! 本模块只保留三件事：
//!
//! 1. 前缀设置的校验与归一化（[`validate_route_prefix`] / [`normalize_route_prefix`]）；
//! 2. session 粘性绑定表（[`RouteBindingStore`]，进程内）；
//! 3. [`apply_session_routing`] 粘性钩子——在 `resolve_stack_target` **之前**调用：
//!    - 短形式 `<prefix><key>` → 该成员默认模型 + 锁定成员 + 绑定 session
//!    - 保留 key `<prefix>default`（claude 形态 `<prefix>claude-default`）→ 解绑 +
//!      本请求显式走默认成员的默认模型
//!    - 全 id（`<prefix>…--model` / `<prefix>key/model`）→ 只绑定 session，
//!      解析与锁定由后续 `resolve_stack_target` 完成
//!    - 裸模型名 → 粘性跟随绑定的成员（subagent / classifier / 后台 haiku），
//!      模型名走该成员的常规模型映射，不透传
//!
//! 审计保真：调用点在 api_log record_received 之后（received 报文保留原文，
//! forward 报文为改写后内容）；request_model（ctx）保留原值，用量归因随
//! ctx.provider 落到锁定成员。

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::app_config::AppType;
use crate::error::AppError;
use crate::live::engine::DeviceStore;
use crate::mode::stack;
use crate::mode::state;
use crate::provider::Provider;
use crate::proxy::error::ProxyError;
use crate::proxy::handler_context::RequestContext;
use crate::proxy::server::ProxyState;

/// 默认模型 id 前缀（fork: 与上游 Stack 一致为 "ccs-"；完整触发串，含边界符。
/// doc/20261009-设计文档-会话路由改造为聚合模式）
pub const DEFAULT_ROUTE_PREFIX: &str = "ccs-";

/// 保留路由 key：任意触发前缀下短形式 `<prefix><key>` 的 key 为 `default` 时
/// 恒为解绑语义，不查成员表、禁止被成员 key 占用
pub const RESERVED_ROUTE_KEY: &str = "default";

/// session 粘性绑定表（进程内，不落库）
///
/// 首个带聚合前缀的请求绑定 session → 成员 key；同 session 的裸模型名请求
/// （subagent / classifier / 后台 haiku，模型名来自 CLAUDE_CODE_SUBAGENT_MODEL
/// 与档位默认值）复用该成员。滚动续期：命中即刷新 last_used；容量上限时
/// 淘汰最久未用（防长期泄漏）。进程重启 = 全部解绑回落默认成员
/// （内存态，无持久化损坏风险）。std 同步锁足够：操作均为 O(1) 纯内存
/// 读写，持锁时间极短且不跨 await。
#[derive(Debug)]
pub struct RouteBindingStore {
    inner: RwLock<HashMap<String, RouteBindingEntry>>,
    capacity: usize,
    ttl: Duration,
}

#[derive(Debug, Clone)]
struct RouteBindingEntry {
    route_key: String,
    last_used: Instant,
}

impl RouteBindingStore {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            capacity,
            ttl,
        }
    }

    /// 绑定（新 key 覆盖旧绑定 = 会话中途换成员）
    pub fn bind(&self, session_id: &str, route_key: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(
                session_id.to_string(),
                RouteBindingEntry {
                    route_key: route_key.to_string(),
                    last_used: Instant::now(),
                },
            );
            Self::evict_if_over_capacity(&mut map, self.capacity);
        }
    }

    /// 查询绑定（命中即续期；过期返回 None 并移除）
    pub fn lookup(&self, session_id: &str) -> Option<String> {
        let mut map = self.inner.write().ok()?;
        match map.get_mut(session_id) {
            Some(entry) => {
                if entry.last_used.elapsed() > self.ttl {
                    map.remove(session_id);
                    None
                } else {
                    entry.last_used = Instant::now();
                    Some(entry.route_key.clone())
                }
            }
            None => None,
        }
    }

    /// 解绑（`<前缀>default` 与绑定失效时调用）
    pub fn unbind(&self, session_id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(session_id);
        }
    }

    /// 容量超限淘汰最久未用条目。绑定操作 O(1)，淘汰 O(n) 但 n ≤ 容量上限
    /// （默认 1000）且仅在插入超限时触发，无性能热点。
    fn evict_if_over_capacity(map: &mut HashMap<String, RouteBindingEntry>, capacity: usize) {
        while map.len() > capacity {
            let Some(oldest) = map
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone())
            else {
                return;
            };
            map.remove(&oldest);
        }
    }
}

impl Default for RouteBindingStore {
    fn default() -> Self {
        Self::new(1000, Duration::from_secs(3600))
    }
}

/// 校验聚合模型 id 前缀（保存设置时 fail-fast）：
/// 非空、1–8 个字符、可打印 ASCII、不含 `:`、
/// 必须以非字母数字字符结尾（自带边界符——裸前缀会把字母开头的
/// 模型名（glm-4.7、gpt-5）误判为聚合寻址并 fail-closed 报错）
pub fn validate_route_prefix(prefix: &str) -> Result<(), String> {
    if prefix.is_empty() {
        return Err("路由前缀不能为空".to_string());
    }
    let char_count = prefix.chars().count();
    if !(1..=8).contains(&char_count) {
        return Err(format!("路由前缀长度须为 1–8 个字符: {prefix}"));
    }
    if !prefix.chars().all(|c| c.is_ascii_graphic()) {
        return Err(format!(
            "路由前缀只能使用可打印 ASCII 字符（不含空白）: {prefix}"
        ));
    }
    if prefix.contains(':') {
        return Err(format!(
            "路由前缀不能包含冒号「:」（它是保留字之外的模型名合法字符，易混淆）: {prefix}"
        ));
    }
    let last = prefix.chars().last().expect("non-empty checked above");
    if last.is_ascii_alphanumeric() {
        return Err(format!(
            "路由前缀必须以非字母数字字符结尾（如「.」「@」「#」「-」）: {prefix}"
        ));
    }
    Ok(())
}

/// 运行时归一化（fail-safe）：settings 值非法（如直接改文件绕过保存校验）时
/// 回退默认前缀并告警。全局基础设施不因设置值非法阻断请求，
/// 区别于成员 key 未命中的 fail-closed。
pub fn normalize_route_prefix(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(prefix) if validate_route_prefix(prefix).is_ok() => prefix.to_string(),
        Some(prefix) => {
            log::warn!(
                "[SessionRouting] 路由前缀设置非法（{prefix:?}），回退默认 {DEFAULT_ROUTE_PREFIX:?}"
            );
            DEFAULT_ROUTE_PREFIX.to_string()
        }
        None => DEFAULT_ROUTE_PREFIX.to_string(),
    }
}

// ============================================================================
// apply_session_routing（粘性钩子）
// ============================================================================

/// 按客户端的协议返回 400 错误体（Claude 用 Anthropic 信封，Codex 用 OpenAI 信封，
/// 与 handlers::stack_miss_body 同款口径）。
fn bad_request(app: &AppType, message: &str) -> Box<Response> {
    let body = match app {
        AppType::Claude => json!({
            "type": "error",
            "error": { "type": "invalid_request_error", "message": message },
        }),
        _ => json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "param": "model",
                "code": "model_not_found",
            }
        }),
    };
    Box::new((StatusCode::BAD_REQUEST, Json(body)).into_response())
}

fn db_error(error: AppError) -> Box<Response> {
    Box::new(ProxyError::DatabaseError(error.to_string()).into_response())
}

/// 粘性跟随（裸模型名请求）：绑定命中 → 锁定到绑定成员，模型名不改写、不透传
/// ——照常走该成员的常规模型映射（claude：档位 → subagent 保护 → ANTHROPIC_MODEL
/// 兜底；codex：model / catalog 链）。绑定失效（成员移除/删除、聚合模式关闭）时
/// 自动解绑回落默认成员并记日志，不报错。
async fn sticky_follow(
    state: &ProxyState,
    ctx: &mut RequestContext,
    store: &DeviceStore,
) -> Result<(), ProxyError> {
    if !ctx.session_client_provided {
        return Ok(()); // 生成型 session id 每请求都变，绑定无意义
    }
    let Some(key) = state.route_bindings.lookup(&ctx.session_id) else {
        return Ok(()); // 无绑定：默认成员原路径
    };
    let stack_on = state::stack_mode(store, ctx.app_type_str)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
    if !stack_on {
        state.route_bindings.unbind(&ctx.session_id);
        log::warn!(
            "[SessionRouting] session {} 绑定存留但聚合模式已关（key: {key}），解绑回落默认成员",
            ctx.session_id
        );
        return Ok(());
    }
    let stack_state = state::stack(store, ctx.app_type_str)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
    match stack::resolve_member(&state.db, &stack_state, &ctx.app_type, &key) {
        Ok(Ok(target)) => {
            // 守卫与显式路由一致（A1）：仅置换 provider 链，不动模型名
            let target_name = target.name.clone();
            lock_context_to_provider(ctx, target);
            log::info!(
                "[SessionRouting] session {} 粘性跟随成员「{target_name}」（key: {key}）",
                ctx.session_id
            );
            Ok(())
        }
        Ok(Err(_miss)) => {
            // 绑定失效（成员移出名单 / 供应商删除）：清绑定、回落默认成员并记日志
            state.route_bindings.unbind(&ctx.session_id);
            log::warn!(
                "[SessionRouting] session {} 绑定的成员已失效（key: {key}），回落默认成员",
                ctx.session_id
            );
            Ok(())
        }
        Err(e) => Err(ProxyError::DatabaseError(e.to_string())),
    }
}

/// 会话粘性钩子（fork：仅 Claude / Codex 链路，在 `resolve_stack_target` 之前调用——
/// 它 Hit 后会把 model 改写为上游名，晚于它就无法再解码绑定/跟随）。
///
/// 1. 短形式 `<prefix><key>` → 该成员默认模型 + 锁定 + 绑定 session（fail-closed：
///    key 未命中报错并列出可用 key；成员无默认模型报错，不静默回落）
/// 2. 保留 key `default` → 解绑；本请求显式走默认成员（proxy_route）的默认模型；
///    无默认成员/默认模型时解绑仍执行、请求报错
/// 3. 全 id → 仅绑定 session（解析与锁定交给后续 resolve_stack_target）
/// 4. 裸模型名 → [`sticky_follow`] 粘性跟随
/// 5. Malformed → 400
pub async fn apply_session_routing(
    state: &ProxyState,
    ctx: &mut RequestContext,
    body: &mut Value,
    store: &DeviceStore,
) -> Result<(), Box<Response>> {
    use stack::Decoded;

    if !matches!(ctx.app_type, AppType::Claude | AppType::Codex) {
        return Ok(()); // claude-desktop 等链路随旧体系退役（设计 D4）
    }
    let Some(model) = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(());
    };
    let prefix = crate::settings::get_route_prefix();
    match stack::decode(&prefix, &ctx.app_type, &model) {
        Decoded::Plain => sticky_follow(state, ctx, store)
            .await
            .map_err(|e| Box::new(e.into_response()) as Box<Response>),
        // 保留 key：解绑 + 本请求显式走默认成员的默认模型（设计 D8）
        Decoded::Short { key } if key.eq_ignore_ascii_case(RESERVED_ROUTE_KEY) => {
            state.route_bindings.unbind(&ctx.session_id);
            log::info!(
                "[SessionRouting] session {} 请求解绑，回落默认成员",
                ctx.session_id
            );
            if !state::stack_mode(store, ctx.app_type_str).map_err(db_error)? {
                return Err(bad_request(
                    &ctx.app_type,
                    &stack::StackMiss::StackOff.message(&model),
                ));
            }
            let mode = state::mode_state(store, ctx.app_type_str).map_err(db_error)?;
            let Some(default_id) = mode.proxy_route else {
                return Err(bad_request(
                    &ctx.app_type,
                    &format!(
                        "聚合的模型 {model} 需要 CC Switch 里配置默认供应商 (No default member is configured for {model})"
                    ),
                ));
            };
            let default_provider = state
                .db
                .get_provider_by_id(&default_id, ctx.app_type_str)
                .map_err(db_error)?;
            let Some(default_provider) = default_provider else {
                return Err(bad_request(
                    &ctx.app_type,
                    &stack::StackMiss::Deleted.message(&model),
                ));
            };
            let Some(upstream) =
                stack::member_default_model(&ctx.app_type, "", &default_provider)
            else {
                return Err(bad_request(
                    &ctx.app_type,
                    &stack::StackMiss::NoDefaultModel.message(&model),
                ));
            };
            body["model"] = Value::String(upstream);
            let target_name = default_provider.name.clone();
            lock_context_to_provider(ctx, default_provider);
            log::info!(
                "[SessionRouting] session {} 解绑后显式走默认成员「{target_name}」默认模型",
                ctx.session_id
            );
            Ok(())
        }
        // 短形式：成员默认模型 + 锁定 + 绑定
        Decoded::Short { key } => {
            if !state::stack_mode(store, ctx.app_type_str).map_err(db_error)? {
                return Err(bad_request(
                    &ctx.app_type,
                    &stack::StackMiss::StackOff.message(&model),
                ));
            }
            let stack_state = state::stack(store, ctx.app_type_str).map_err(db_error)?;
            let target = match stack::resolve_member(&state.db, &stack_state, &ctx.app_type, &key)
            {
                Ok(Ok(target)) => target,
                Ok(Err(miss)) => {
                    let mut message = miss.message(&model);
                    if matches!(miss, stack::StackMiss::Unknown) {
                        // fail-closed 报错附可用 key 列表（设计 §10，沿用旧报错形态）
                        let available = stack::available_keys(&stack_state);
                        message.push_str(&format!(
                            "（当前可用: {}）",
                            if available.is_empty() {
                                "无".to_string()
                            } else {
                                available.join(", ")
                            }
                        ));
                    }
                    return Err(bad_request(&ctx.app_type, &message));
                }
                Err(e) => return Err(db_error(e)),
            };
            let Some(upstream) = stack::member_default_model(&ctx.app_type, &key, &target)
            else {
                return Err(bad_request(
                    &ctx.app_type,
                    &stack::StackMiss::NoDefaultModel.message(&model),
                ));
            };
            if ctx.session_client_provided {
                state.route_bindings.bind(&ctx.session_id, &key);
            }
            body["model"] = Value::String(upstream);
            let target_name = target.name.clone();
            lock_context_to_provider(ctx, target);
            log::info!(
                "[SessionRouting] session {} 短形式 key「{key}」→ 成员「{target_name}」默认模型",
                ctx.session_id
            );
            Ok(())
        }
        // 全 id：解析与锁定由后续 resolve_stack_target 完成；这里只做尽力绑定
        // （key 解不出来时不绑，真正的 400 由 resolve_stack_target 报）
        Decoded::Stack { key, .. } => {
            if ctx.session_client_provided
                && state::stack_mode(store, ctx.app_type_str).map_err(db_error)?
            {
                let stack_state = state::stack(store, ctx.app_type_str).map_err(db_error)?;
                if matches!(
                    stack::resolve_member(&state.db, &stack_state, &ctx.app_type, &key),
                    Ok(Ok(_))
                ) {
                    state.route_bindings.bind(&ctx.session_id, &key);
                }
            }
            Ok(())
        }
        Decoded::Malformed => Err(bad_request(
            &ctx.app_type,
            &stack::StackMiss::Unknown.message(&model),
        )),
    }
}

/// A1 守卫（沿用旧设计 §3.4）：把 ctx 锁定到目标成员——provider / providers
///（单元素）/ current_provider_id 全部指向锁定成员，使 forwarder 4 处
/// `should_switch` 恒为 false：不偷换默认成员、不污染 failover_count、
/// 不触发 try_switch。单元素 Vec 同时天然绕过熔断放行检查，
/// 显式点名不应被全局健康度拦截；record_failure 健康统计仍照常累计。
fn lock_context_to_provider(ctx: &mut RequestContext, target: Provider) {
    ctx.current_provider_id = target.id.clone();
    ctx.provider = target.clone();
    ctx.set_providers(vec![target]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_prefixed_triggers() {
        assert!(validate_route_prefix("G.").is_ok());
        assert!(validate_route_prefix("@").is_ok());
        assert!(validate_route_prefix("##").is_ok());
        assert!(validate_route_prefix("R.").is_ok());
        assert!(validate_route_prefix("ccs-").is_ok());
    }

    #[test]
    fn validate_rejects_bare_alnum_ending() {
        // 裸字母数字结尾会把 glm-4.7 / gpt-5 等误判为路由请求
        assert!(validate_route_prefix("G").is_err());
        assert!(validate_route_prefix("go").is_err());
    }

    #[test]
    fn validate_rejects_colon_whitespace_long_and_non_ascii() {
        assert!(validate_route_prefix("").is_err());
        assert!(validate_route_prefix("G :").is_err());
        assert!(validate_route_prefix("G. ").is_err());
        assert!(validate_route_prefix("toolongpfx.").is_err()); // 11 字符超上限
        assert!(validate_route_prefix("路.").is_err()); // 非 ASCII
    }

    #[test]
    fn normalize_falls_back_to_default_on_invalid() {
        // fork: 默认前缀随聚合模式对齐上游（"G." → "ccs-"）
        assert_eq!(normalize_route_prefix(None), "ccs-");
        assert_eq!(normalize_route_prefix(Some("")), "ccs-");
        assert_eq!(normalize_route_prefix(Some("  ")), "ccs-");
        assert_eq!(normalize_route_prefix(Some("G")), "ccs-"); // 裸字母结尾（改库绕过校验）
        assert_eq!(normalize_route_prefix(Some("@")), "@");
    }

    use std::time::Duration;

    #[test]
    fn bind_lookup_and_rebind_override() {
        let store = super::RouteBindingStore::new(10, Duration::from_secs(60));
        assert_eq!(store.lookup("s1"), None);
        store.bind("s1", "ds");
        assert_eq!(store.lookup("s1"), Some("ds".to_string()));
        // 新 key 覆盖旧绑定（会话中途换成员）
        store.bind("s1", "glm");
        assert_eq!(store.lookup("s1"), Some("glm".to_string()));
    }

    #[test]
    fn lookup_expires_after_ttl() {
        let store = super::RouteBindingStore::new(10, Duration::from_millis(50));
        store.bind("s1", "ds");
        assert_eq!(store.lookup("s1"), Some("ds".to_string())); // 命中即续期
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(store.lookup("s1"), None); // 过期移除
        assert_eq!(store.lookup("s1"), None); // 移除后不再复活
    }

    #[test]
    fn unbind_removes_binding() {
        let store = super::RouteBindingStore::default();
        store.bind("s1", "ds");
        store.unbind("s1");
        assert_eq!(store.lookup("s1"), None);
        store.unbind("s1"); // 幂等
    }

    #[test]
    fn capacity_evicts_least_recently_used() {
        let store = super::RouteBindingStore::new(2, Duration::from_secs(60));
        store.bind("s1", "a");
        store.bind("s2", "b");
        std::thread::sleep(Duration::from_millis(10));
        store.lookup("s1"); // s1 续期 → s2 成为最旧
        store.bind("s3", "c"); // 超容量，淘汰 s2
        assert_eq!(store.lookup("s1"), Some("a".to_string()));
        assert_eq!(store.lookup("s2"), None);
        assert_eq!(store.lookup("s3"), Some("c".to_string()));
    }

    // ==========================================================================
    // apply_session_routing（fork Task 3）
    // ==========================================================================

    use crate::database::Database;
    use http_body_util::BodyExt;
    use std::sync::Arc;

    fn env_provider(id: &str, env: serde_json::Value) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            format!("P-{id}"),
            serde_json::json!({ "env": env }),
            None,
        );
        provider.icon = Some(id.to_string());
        provider
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: DeviceStore,
        state: ProxyState,
    }

    /// claude 聚合模式：成员 kimi（默认模型 kimi-k3）、zhipu（glm-5.2[1M]）；
    /// 默认成员（proxy_route）= kimi。bare 按需追加（无默认模型用例）。
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(dir.path());
        let state = ProxyState::for_test(Arc::new(Database::memory().unwrap()));
        for row in [
            env_provider("kimi", serde_json::json!({ "ANTHROPIC_MODEL": "kimi-k3" })),
            env_provider("zhipu", serde_json::json!({ "ANTHROPIC_MODEL": "glm-5.2[1M]" })),
        ] {
            state.db.save_provider("claude", &row).unwrap();
        }
        state
            .db
            .set_current_provider("claude", "kimi")
            .unwrap();
        state::update(&store, |live| {
            let claude = live.apps.entry("claude".to_string()).or_default();
            claude.mode = Some(state::Mode::Proxy);
            claude.proxy_route = Some("kimi".to_string());
            let stack = &mut claude.stack;
            stack.enabled = true;
            stack.members = ["kimi", "zhipu"].map(str::to_string).to_vec();
            stack.keys.insert("kimi".to_string(), "kimi".to_string());
            stack.keys.insert("zhipu".to_string(), "zhipu".to_string());
        })
        .unwrap();
        Fixture {
            _dir: dir,
            store,
            state,
        }
    }

    /// 前缀动态取自设置（与 server.rs 路由测试同款口径，本机自定义前缀时不脆弱）
    fn prefix() -> String {
        crate::settings::get_route_prefix()
    }

    async fn make_ctx(fx: &Fixture, app: AppType, model: &str) -> RequestContext {
        let body = serde_json::json!({ "model": model, "messages": [] });
        let app_str: &'static str = match app {
            AppType::Claude => "claude",
            AppType::ClaudeDesktop => "claude-desktop",
            AppType::Codex => "codex",
            _ => "claude",
        };
        let mut ctx = RequestContext::new(
            &fx.state,
            &body,
            &axum::http::HeaderMap::new(),
            app.clone(),
            "Test",
            app_str,
            None,
        )
        .await
        .unwrap();
        ctx.session_id = "s-test".to_string();
        ctx.session_client_provided = true;
        ctx
    }

    #[tokio::test]
    async fn short_form_routes_to_member_default_model() {
        let fx = fixture();
        let id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &id).await;
        let mut body = serde_json::json!({ "model": id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(body["model"], "glm-5.2[1M]");
        assert_eq!(ctx.provider.id, "zhipu");
        assert_eq!(
            fx.state.route_bindings.lookup("s-test"),
            Some("zhipu".to_string())
        );
    }

    #[tokio::test]
    async fn short_form_unknown_key_fails_closed_with_list() {
        let fx = fixture();
        let id = stack::encode_short(&prefix(), &AppType::Claude, "nope");
        let mut ctx = make_ctx(&fx, AppType::Claude, &id).await;
        let mut body = serde_json::json!({ "model": id, "messages": [] });
        let err = apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap_err();
        let response = *err;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // 失败响应体里应含可用 key 列表（读取 body 文本）
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        assert!(text.contains("kimi") && text.contains("zhipu"), "{text}");
    }

    #[tokio::test]
    async fn default_unbinds_and_routes_to_default_member() {
        let fx = fixture();
        // 先绑定 zhipu，再解绑 → 默认成员 kimi 的默认模型
        let zhipu_id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &zhipu_id).await;
        let mut body = serde_json::json!({ "model": zhipu_id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert!(fx.state.route_bindings.lookup("s-test").is_some());

        let default_id = stack::encode_short(&prefix(), &AppType::Claude, "default");
        let mut body = serde_json::json!({ "model": default_id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(fx.state.route_bindings.lookup("s-test"), None);
        assert_eq!(body["model"], "kimi-k3");
        assert_eq!(ctx.provider.id, "kimi");
    }

    #[tokio::test]
    async fn default_without_default_model_errors_but_unbinds() {
        let mut fx = fixture();
        // 把默认成员换成没有任何模型的 bare → 解绑执行、请求 400
        let bare = env_provider("bare", serde_json::json!({}));
        fx.state.db.save_provider("claude", &bare).unwrap();
        state::update(&fx.store, |live| {
            let claude = live.apps.get_mut("claude").unwrap();
            claude.proxy_route = Some("bare".to_string());
            let stack = &mut claude.stack;
            stack.members.push("bare".to_string());
            stack.keys.insert("bare".to_string(), "bare".to_string());
        })
        .unwrap();
        fx.state
            .db
            .set_current_provider("claude", "bare")
            .unwrap();

        let zhipu_id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&mut fx, AppType::Claude, &zhipu_id).await;
        let mut body = serde_json::json!({ "model": zhipu_id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert!(fx.state.route_bindings.lookup("s-test").is_some());

        let default_id = stack::encode_short(&prefix(), &AppType::Claude, "default");
        let mut body = serde_json::json!({ "model": default_id, "messages": [] });
        let err = apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap_err();
        assert_eq!((*err).status(), StatusCode::BAD_REQUEST);
        // 解绑仍执行
        assert_eq!(fx.state.route_bindings.lookup("s-test"), None);
    }

    #[tokio::test]
    async fn sticky_follow_locks_bound_member() {
        let fx = fixture();
        // 先绑定 zhipu
        let zhipu_id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &zhipu_id).await;
        let mut body = serde_json::json!({ "model": zhipu_id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();

        // 同 session 裸模型名 → 锁定绑定成员，模型名不改写
        let mut body = serde_json::json!({ "model": "haiku", "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(ctx.provider.id, "zhipu");
        assert_eq!(body["model"], "haiku");
    }

    #[tokio::test]
    async fn sticky_member_removed_falls_back() {
        let fx = fixture();
        let zhipu_id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &zhipu_id).await;
        let mut body = serde_json::json!({ "model": zhipu_id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();

        // zhipu 移出名单（登记簿保留）→ 绑定失效自动解绑，回落默认成员、不报错。
        // 真实流程每个请求新建 ctx（默认链），这里同样用新 ctx 发裸名请求
        state::update(&fx.store, |live| {
            let stack = &mut live.apps.get_mut("claude").unwrap().stack;
            stack.members.retain(|id| id != "zhipu");
        })
        .unwrap();
        let mut ctx = make_ctx(&fx, AppType::Claude, "haiku").await;
        let mut body = serde_json::json!({ "model": "haiku", "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(fx.state.route_bindings.lookup("s-test"), None);
        assert_eq!(ctx.provider.id, "kimi"); // 默认成员（RequestContext 初始链）
    }

    #[tokio::test]
    async fn full_id_binds_without_rewriting() {
        let fx = fixture();
        let id = stack::encode(&prefix(), &AppType::Claude, "zhipu", "glm-5.2", true);
        let mut ctx = make_ctx(&fx, AppType::Claude, &id).await;
        let mut body = serde_json::json!({ "model": id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        // 全 id 不改写、不锁定（由 resolve_stack_target 负责），只绑定
        assert_eq!(body["model"], id);
        assert_eq!(
            fx.state.route_bindings.lookup("s-test"),
            Some("zhipu".to_string())
        );
    }

    #[tokio::test]
    async fn generated_session_id_never_binds() {
        let fx = fixture();
        let id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &id).await;
        ctx.session_client_provided = false; // 生成型 session id
        let mut body = serde_json::json!({ "model": id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(fx.state.route_bindings.lookup("s-test"), None);
    }

    #[tokio::test]
    async fn non_stack_app_is_noop() {
        let fx = fixture();
        let id = stack::encode_short(&prefix(), &AppType::Claude, "zhipu");
        let mut ctx = make_ctx(&fx, AppType::Claude, &id).await;
        ctx.app_type = AppType::ClaudeDesktop;
        ctx.app_type_str = "claude-desktop";
        let mut body = serde_json::json!({ "model": id, "messages": [] });
        apply_session_routing(&fx.state, &mut ctx, &mut body, &fx.store)
            .await
            .unwrap();
        assert_eq!(body["model"], id);
        assert_eq!(fx.state.route_bindings.lookup("s-test"), None);
    }
}
