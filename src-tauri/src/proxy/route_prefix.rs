//! 会话级模型路由（前缀触发）模块
//!
//! 按请求 model 值的前缀（默认 `G.`，可配置）将请求路由到指定分组：
//! - `G.<key>`         → 路由到 key 分组，使用该分组默认模型（ANTHROPIC_MODEL）
//! - `G.<key>:<model>`  → 路由到 key 分组，显式模型透传（不落默认兜底）
//! - `<prefix>default`  → 解绑 session 粘性绑定，回落默认分组
//!
//! 设计文档：doc/20260908-会话级模型路由.md

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::proxy::error::ProxyError;
use crate::proxy::handler_context::RequestContext;
use crate::proxy::model_mapper::strip_one_m_suffix_for_upstream;
use crate::proxy::server::ProxyState;
use serde_json::Value;

/// 默认路由触发前缀（完整触发串，含边界符）
pub const DEFAULT_ROUTE_PREFIX: &str = "G.";

/// 保留路由 key：任意触发前缀下 `<prefix>default` 恒为解绑语义，
/// 不查分组表、禁止被分组占用
pub const RESERVED_ROUTE_KEY: &str = "default";

/// 解析结果。key 保留原始大小写（匹配层大小写不敏感）；
/// model_override 为 `key:model` 中第一个 `:` 之后的部分，保留原始大小写
/// （模型名透传必须保真，不做大小写改写）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRoute {
    pub key: String,
    pub model_override: Option<String>,
}

/// session 粘性路由绑定表（进程内，不落库，设计 §3.8）
///
/// 首个带路由前缀的请求绑定 session → route_key；同 session 的无前缀请求
/// （subagent / classifier / 后台 haiku，模型名来自 CLAUDE_CODE_SUBAGENT_MODEL
/// 与档位默认值）复用该分组。滚动续期：命中即刷新 last_used；容量上限时
/// 淘汰最久未用（防长期泄漏）。进程重启 = 全部解绑回落默认分组
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

    /// 绑定（新 key 覆盖旧绑定 = 会话中途换分组）
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

    /// 解绑（`<前缀>default` 与分组失效时调用）
    pub fn unbind(&self, session_id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(session_id);
        }
    }

    /// 容量超限淘汰最久未用条目。绑定操作 O(1)，淘汰 O(n) 但 n ≤ 容量上限
    /// （默认 1000）且仅在插入超限时触发，无性能热点。
    fn evict_if_over_capacity(
        map: &mut HashMap<String, RouteBindingEntry>,
        capacity: usize,
    ) {
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

/// 校验路由触发前缀（保存设置时 fail-fast，设计 §3.9）：
/// 非空、1–8 个字符、可打印 ASCII、不含 `:`、
/// 必须以非字母数字字符结尾（自带边界符——裸前缀会把字母开头的
/// 模型名（glm-4.7、gpt-5）误判为路由请求并 fail-closed 报错）
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
            "路由前缀不能包含冒号「:」（它是 key 与模型名的分隔符）: {prefix}"
        ));
    }
    let last = prefix.chars().last().expect("non-empty checked above");
    if last.is_ascii_alphanumeric() {
        return Err(format!(
            "路由前缀必须以非字母数字字符结尾（如「.」「@」「#」）: {prefix}"
        ));
    }
    Ok(())
}

/// 运行时归一化（fail-safe）：DB 值非法（如直接改库绕过保存校验）时
/// 回退默认前缀并告警。全局基础设施不因设置值非法阻断请求，
/// 区别于路由 key 未命中的 fail-closed。
pub fn normalize_route_prefix(raw: Option<&str>) -> String {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(prefix) if validate_route_prefix(prefix).is_ok() => prefix.to_string(),
        Some(prefix) => {
            log::warn!(
                "[RoutePrefix] 路由前缀设置非法（{prefix:?}），回退默认 {DEFAULT_ROUTE_PREFIX:?}"
            );
            DEFAULT_ROUTE_PREFIX.to_string()
        }
        None => DEFAULT_ROUTE_PREFIX.to_string(),
    }
}

/// 解析 model 值是否携带路由前缀（设计 §3.1）：
/// - None：无前缀，走默认分组 / session 粘性绑定（行为零变化）
/// - Some：前缀命中。key 为空（仅前缀本身）时交给 key 匹配层 fail-closed
///
/// 前缀匹配大小写不敏感（宽容 CLI 对 model 串的大小写改写）；
/// `[1M]` 后缀容忍两种形态；按第一个 `:` 切分 key 与模型名
/// （key 与模型名都可含点，点无法无歧义切分）。
pub fn parse_route_target(model: &str, prefix: &str) -> Option<ParsedRoute> {
    if !model.to_lowercase().starts_with(&prefix.to_lowercase()) {
        return None;
    }
    // 前缀经校验/回退后为可打印 ASCII（单字节），按字节长度切片安全；
    // get() 仅作 char 边界防御
    let rest = model.get(prefix.len()..).unwrap_or("");
    let rest = strip_one_m_suffix_for_upstream(rest).trim();
    match rest.split_once(':') {
        Some((key, model_override)) => Some(ParsedRoute {
            key: key.trim().to_string(),
            model_override: (!model_override.trim().is_empty())
                .then(|| model_override.trim().to_string()),
        }),
        None => Some(ParsedRoute {
            key: rest.to_string(),
            model_override: None,
        }),
    }
}

/// per-request 透传标记：`G.<key>:<model>` 显式模型透传。
/// apply_model_mapping 据此跳过目标分组 ANTHROPIC_MODEL 默认兜底
///（模型名是用户点名的真值，静默换成默认模型比上游报错更难排查）
#[derive(Debug, Clone, Copy)]
pub struct RoutePassthrough;

/// 在同 app 全部分组中按路由 key 解析锁定分组（设计 §3.2/§3.4）：
/// - 仅认 route_enabled = true 且 key 匹配（大小写不敏感）的分组
/// - key 重复（改库绕过保存校验）时取 sort_index 最小者并 warn（运行时兜底）
/// - 未命中 fail-closed：报错含当前可用 key 列表，不回落默认分组
pub fn resolve_route_provider(
    all: &indexmap::IndexMap<String, Provider>,
    key: &str,
) -> Result<Provider, ProxyError> {
    let mut candidates: Vec<&Provider> = all
        .values()
        .filter(|p| p.meta.as_ref().and_then(|m| m.route_enabled) == Some(true))
        .filter(|p| {
            p.meta
                .as_ref()
                .and_then(|m| m.route_key.as_deref())
                .map(|k| k.trim().eq_ignore_ascii_case(key.trim()))
                .unwrap_or(false)
        })
        .collect();

    match candidates.len() {
        0 => {
            let available: Vec<String> = all
                .values()
                .filter(|p| {
                    p.meta.as_ref().and_then(|m| m.route_enabled) == Some(true)
                })
                .filter_map(|p| p.meta.as_ref().and_then(|m| m.route_key.clone()))
                .collect();
            Err(ProxyError::ConfigError(format!(
                "路由 key「{key}」未匹配到已加入路由的分组（当前可用: {}）",
                if available.is_empty() {
                    "无".to_string()
                } else {
                    available.join(", ")
                }
            )))
        }
        1 => Ok(candidates.remove(0).clone()),
        _ => {
            candidates.sort_by_key(|p| p.sort_index.unwrap_or(usize::MAX));
            let chosen = candidates[0].clone();
            log::warn!(
                "[RoutePrefix] 路由 key「{key}」命中多个分组，兜底取 sort_index 最小者: {}",
                chosen.name
            );
            Ok(chosen)
        }
    }
}

/// session 粘性路由（设计 §3.8）：同 session 的无前缀请求
/// （subagent / classifier / 后台 haiku，模型名来自
/// CLAUDE_CODE_SUBAGENT_MODEL 与档位默认值，不带前缀）复用首个
/// `G.<key>` 请求绑定的分组，避免「主对话在锁定分组、子代理在默认分组」
/// 的会话内分裂。
///
/// 绑定命中 ≠ 显式路由：不改写 body.model、不插透传标记——模型名照常
/// 走目标分组常规 map_model（档位 → subagent 保护 → ANTHROPIC_MODEL 兜底）。
/// 目标分组配了 ANTHROPIC_MODEL 时可把 CLAUDE_CODE_SUBAGENT_MODEL 的值
/// （如 deepseek-xxx）兜底替换，避免发给不认识它的上游报错；未配兜底则
/// 原样发出（与无前缀请求行为一致）。
async fn sticky_route_lookup(
    state: &ProxyState,
    ctx: &mut RequestContext,
) -> Result<(), ProxyError> {
    if !ctx.session_client_provided {
        return Ok(()); // 生成型 session id 每请求都变，绑定无意义
    }
    let Some(key) = state.route_bindings.lookup(&ctx.session_id) else {
        return Ok(()); // 无绑定：默认分组原路径
    };
    let all = state
        .db
        .get_all_providers(ctx.app_type_str)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
    match resolve_route_provider(&all, &key) {
        Ok(target) => {
            // 守卫与显式路由一致（A1）：仅置换 provider 链，不动模型名
            let target_name = target.name.clone();
            lock_context_to_provider(ctx, target);
            log::debug!(
                "[RoutePrefix] session {} 粘性跟随分组「{target_name}」（key: {key}）",
                ctx.session_id
            );
            Ok(())
        }
        Err(_) => {
            // 绑定失效（key 对应分组被删 / 路由开关关闭）：
            // 清绑定、回落默认分组并记日志（设计 §3.8 绑定失效）
            state.route_bindings.unbind(&ctx.session_id);
            log::warn!(
                "[RoutePrefix] session {} 绑定的路由 key「{key}」已失效（分组删除或路由关闭），回落默认分组",
                ctx.session_id
            );
            Ok(())
        }
    }
}

/// 会话级路由应用点（仅 Claude / ClaudeDesktop 链路调用，设计 §3.3/§3.4）：
/// 1. model 带路由前缀 → 解析 key、锁定分组、改写 body.model、绑定 session
/// 2. `<前缀>default` → 解绑 session，回落默认分组
/// 3. model 无前缀 → session 粘性查询（sticky_route_lookup，跟随绑定分组）
///
/// 审计保真：调用点在 api_log record_received 之后（received 报文保留
/// `G.` 原文，forward 报文为改写后内容）；request_model（ctx）保留原值，
/// 用量归因随 ctx.provider 落到锁定分组。
pub async fn apply_route(
    state: &ProxyState,
    ctx: &mut RequestContext,
    body: &mut Value,
    extensions: &mut axum::http::Extensions,
) -> Result<(), ProxyError> {
    if !matches!(ctx.app_type, AppType::Claude | AppType::ClaudeDesktop) {
        return Ok(()); // 生效范围守卫（设计 §3.7，双保险）
    }
    let Some(model) = body.get("model").and_then(Value::as_str).map(str::to_string) else {
        return Ok(());
    };
    let prefix = crate::settings::get_route_prefix();
    let Some(parsed) = parse_route_target(&model, &prefix) else {
        return sticky_route_lookup(state, ctx).await;
    };
    if parsed.key.eq_ignore_ascii_case(RESERVED_ROUTE_KEY) {
        state.route_bindings.unbind(&ctx.session_id);
        log::info!(
            "[RoutePrefix] session {} 请求解绑路由，回落默认分组",
            ctx.session_id
        );
        return Ok(());
    }
    let all = state
        .db
        .get_all_providers(ctx.app_type_str)
        .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
    let target = resolve_route_provider(&all, &parsed.key)?;
    // 仅客户端提供的 session id 才绑定（生成的 UUID 每请求都变，绑了也白绑）
    if ctx.session_client_provided {
        state.route_bindings.bind(&ctx.session_id, &parsed.key);
    }
    match parsed.model_override.as_deref() {
        Some(model_override) => {
            // 显式模型透传：写入原始值 + 标记跳过 ANTHROPIC_MODEL 兜底
            body["model"] = Value::String(model_override.to_string());
            extensions.insert(RoutePassthrough);
        }
        None => {
            let default_model = target
                .settings_config
                .pointer("/env/ANTHROPIC_MODEL")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|m| !m.is_empty());
            let Some(default_model) = default_model else {
                return Err(ProxyError::ConfigError(format!(
                    "路由分组「{}」未配置默认模型（env.ANTHROPIC_MODEL），无法处理无显式模型的路由请求",
                    target.name
                )));
            };
            // 不插 RoutePassthrough：置换值照常走目标分组 map_model——
            // 若值命中档位子串（如 claude-sonnet-4-6）且分组另配档位模型，
            // 会被再替换一次；结果仍属该分组的已配置模型，接受（设计 §3.3 备注）
            body["model"] = Value::String(default_model);
        }
    }
    let target_name = target.name.clone();
    lock_context_to_provider(ctx, target);
    log::info!(
        "[RoutePrefix] session {} 路由 key「{}」→ 分组「{target_name}」",
        ctx.session_id,
        parsed.key
    );
    Ok(())
}

/// A1 守卫（设计 §3.4）：把 ctx 锁定到目标分组——provider / providers
///（单元素）/ current_provider_id 全部指向锁定分组，使 forwarder 4 处
/// `should_switch`（forwarder.rs:565/668/814/978）恒为 false：不偷换默认
/// 分组、不污染 failover_count、不触发 try_switch。
/// 单元素 Vec 同时天然绕过熔断放行检查（forwarder.rs:465），显式点名
/// 不应被全局健康度拦截；record_failure 健康统计仍照常累计（A2）。
/// 已知可接受残留：状态栏「当前分组」展示字段（forwarder.rs:554
/// current_providers.insert）无守卫，路由期间临时显示路由目标，
/// 下个普通请求即刷回（设计 §3.4）。
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
    fn parse_returns_none_without_prefix() {
        assert_eq!(parse_route_target("sonnet", "G."), None);
        assert_eq!(parse_route_target("glm-4.7", "G."), None);
        assert_eq!(parse_route_target("", "G."), None);
        // 误匹配防线：裸前缀 "G." 不会命中 "sonnet"
        assert_eq!(parse_route_target("claude-opus-4-8", "@"), None);
    }

    #[test]
    fn parse_key_only_and_key_model() {
        assert_eq!(
            parse_route_target("G.ds", "G."),
            Some(ParsedRoute { key: "ds".into(), model_override: None })
        );
        assert_eq!(
            parse_route_target("G.ds:claude-opus-4-8", "G."),
            Some(ParsedRoute { key: "ds".into(), model_override: Some("claude-opus-4-8".into()) })
        );
        // key 与模型名都可含点，按第一个 `:` 切分
        assert_eq!(
            parse_route_target("G.mini:MiniMax-M2.7-highspeed", "G."),
            Some(ParsedRoute { key: "mini".into(), model_override: Some("MiniMax-M2.7-highspeed".into()) })
        );
        // 冒号后为空视同无显式模型（分组默认模型）
        assert_eq!(
            parse_route_target("G.ds:", "G."),
            Some(ParsedRoute { key: "ds".into(), model_override: None })
        );
        // key 可含点与连字符
        assert_eq!(
            parse_route_target("G.my-key.v2:sonnet", "G."),
            Some(ParsedRoute { key: "my-key.v2".into(), model_override: Some("sonnet".into()) })
        );
    }

    #[test]
    fn parse_is_case_insensitive_on_prefix_and_keeps_model_case() {
        assert_eq!(
            parse_route_target("g.DS:DeepSeek-R1", "G."),
            Some(ParsedRoute { key: "DS".into(), model_override: Some("DeepSeek-R1".into()) })
        );
        // 自定义前缀
        assert_eq!(
            parse_route_target("@ds", "@"),
            Some(ParsedRoute { key: "ds".into(), model_override: None })
        );
    }

    #[test]
    fn parse_tolerates_one_m_suffix_both_forms() {
        // [1M] 可能被 Claude Code 剥离后再到达代理，两种形态都容忍
        assert_eq!(
            parse_route_target("G.ds[1M]", "G."),
            Some(ParsedRoute { key: "ds".into(), model_override: None })
        );
        assert_eq!(
            parse_route_target("G.ds:sonnet[1M]", "G."),
            Some(ParsedRoute { key: "ds".into(), model_override: Some("sonnet".into()) })
        );
    }

    #[test]
    fn parse_reserved_default_and_empty_key() {
        // 保留 key：解析层不特殊处理，由 apply_route 判定解绑语义
        assert_eq!(
            parse_route_target("G.default", "G."),
            Some(ParsedRoute { key: "default".into(), model_override: None })
        );
        assert_eq!(
            parse_route_target("@default", "@"),
            Some(ParsedRoute { key: "default".into(), model_override: None })
        );
        // 仅前缀本身（key 空）：交给 key 匹配层 fail-closed（报 key 未命中）
        assert_eq!(
            parse_route_target("G.", "G."),
            Some(ParsedRoute { key: "".into(), model_override: None })
        );
    }

    #[test]
    fn normalize_falls_back_to_default_on_invalid() {
        assert_eq!(normalize_route_prefix(None), "G.");
        assert_eq!(normalize_route_prefix(Some("")), "G.");
        assert_eq!(normalize_route_prefix(Some("  ")), "G.");
        assert_eq!(normalize_route_prefix(Some("G")), "G."); // 裸字母结尾（改库绕过校验）
        assert_eq!(normalize_route_prefix(Some("@")), "@");
    }

    use std::time::Duration;

    #[test]
    fn bind_lookup_and_rebind_override() {
        let store = super::RouteBindingStore::new(10, Duration::from_secs(60));
        assert_eq!(store.lookup("s1"), None);
        store.bind("s1", "ds");
        assert_eq!(store.lookup("s1"), Some("ds".to_string()));
        // 新 key 覆盖旧绑定（会话中途换分组）
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

    use crate::provider::{Provider, ProviderMeta};
    use indexmap::IndexMap;

    fn routed_provider(id: &str, key: &str, sort_index: Option<usize>) -> Provider {
        let mut p = Provider::with_id(
            id.to_string(),
            format!("P-{id}"),
            serde_json::json!({"env": {"ANTHROPIC_BASE_URL": "https://example.com"}}),
            None,
        );
        p.sort_index = sort_index;
        p.meta = Some(ProviderMeta {
            route_enabled: Some(true),
            route_key: Some(key.to_string()),
            ..Default::default()
        });
        p
    }

    fn all_providers(entries: Vec<Provider>) -> IndexMap<String, Provider> {
        entries
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect()
    }

    #[test]
    fn resolve_matches_key_case_insensitively() {
        let all = all_providers(vec![routed_provider("a", "ds", None)]);
        let hit = resolve_route_provider(&all, "DS").expect("case-insensitive hit");
        assert_eq!(hit.id, "a");
    }

    #[test]
    fn resolve_ignores_disabled_and_keyless_providers() {
        let mut disabled = routed_provider("a", "ds", None);
        disabled.meta = Some(ProviderMeta {
            route_enabled: Some(false),
            route_key: Some("ds".into()),
            ..Default::default()
        });
        let mut keyless = routed_provider("b", "", None);
        keyless.meta = Some(ProviderMeta {
            route_enabled: Some(true),
            route_key: None,
            ..Default::default()
        });
        let all = all_providers(vec![disabled, keyless]);
        assert!(resolve_route_provider(&all, "ds").is_err());
    }

    #[test]
    fn resolve_duplicate_key_falls_back_to_smallest_sort_index() {
        // 直接改库绕过保存校验的场景：运行时兜底取 sort_index 最小者
        let all = all_providers(vec![
            routed_provider("later", "ds", Some(5)),
            routed_provider("first", "ds", Some(1)),
        ]);
        let hit = resolve_route_provider(&all, "ds").expect("fallback hit");
        assert_eq!(hit.id, "first");
    }

    #[test]
    fn resolve_missing_key_fails_closed_with_available_list() {
        let all = all_providers(vec![
            routed_provider("a", "ds", None),
            routed_provider("b", "glm", None),
        ]);
        let err = resolve_route_provider(&all, "notexist").expect_err("fail-closed");
        let msg = err.to_string();
        assert!(msg.contains("notexist"), "msg: {msg}");
        assert!(msg.contains("ds") && msg.contains("glm"), "可用 key 列表缺失: {msg}");
    }
}
