//! Stack 模式（界面和代码都叫 Stack）。
//!
//! Stack 模式和路由模式在界面上二选一，内部都是代理模式：Stack 模式多一个开关位
//! （[`StackState::enabled`]）。Stack 模式下供应商列表是累加式的：添加的每一家（第三方）的
//! 模型以带保留前缀的 id 发布给客户端，选中后请求直达那一家；不带前缀的请求发往「默认」
//! 那家（代理路由），不做故障转移。Claude Code 的四档别名（启动默认、后台任务、子代理别名）
//! 未配映射时都指向默认那家列表里的第一个模型，配了档位映射的档用映射到的模型
//! （[`claude_route_roles`]），平时用哪个由用户在 `/model` 里选。
//!
//! - 名单和 key 登记簿存在 `live-state.json`（[`StackState`]），增删和客户端文件在同一个
//!   操作里提交（`controller::set_stack_member`）；默认那家也在名单里，不能移除；
//! - key 一经分配永久归这家（[`allocate_key`]）：客户端会一直带着选中过的 id，key 改了
//!   指向，旧 id 就会被悄悄发到另一家；
//! - 带保留前缀的 id 解不出来（不在 Stack 模式、成员已移除、供应商已删除、key 没登记）一律
//!   报错，不回落到默认路由（[`resolve`]）：回落会用别家的钱、别家的模型回答，用户看不出来；
//! - Stack 请求不读也不写任何路由状态（熔断器、故障转移、「正在使用」、代理统计），见
//!   `proxy::forwarder` 的 `routing_state_enabled`。

use serde::Serialize;
use serde_json::{Map, Value};

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::live::engine::DeviceStore;
use crate::live::project::claude::{env_string, has_one_m_marker, ONE_M_MARKER_FOR_CLIENT};
use crate::provider::{ClaudeStackModel, Provider};
use crate::proxy::model_mapper::strip_one_m_suffix_for_upstream;
use crate::services::provider::codex_client_catalog::StaleClients;

use super::state::{self, StackState};

// fork: 模型 id 前缀可配置（会话路由改造为聚合模式，doc/20261009-设计文档-会话路由改造为聚合模式）。
// 上游默认前缀是 `ccs-`；用户可经设置改成 G. 等。生产调用方统一经
// [`configured_prefix`] 取值一次显式传入——不在这层读全局 settings：settings_store 是 OnceLock
// 惰性读真实磁盘，掺进纯函数会让测试结果机器相关（开发机配置了 G. 就全挂）。
/// 配置的模型 id 前缀（settings 缺省/非法时回退上游默认，见 `route_prefix::DEFAULT_ROUTE_PREFIX`）。
pub fn configured_prefix() -> String {
    crate::settings::get_route_prefix()
}

/// Claude Code 的 Stack 模型 id：`<prefix>claude-<key>--<model>`。id 里要有 `claude` 才进
/// `/model` 选择器，不以 `claude-` 开头 MAX 窗口才生效（fork：前缀可配置，上游写死的
/// `ccs-claude-` 拆成 `<prefix>` + 这个中缀）。
const CLAUDE_INFIX: &str = "claude-";
const CLAUDE_SEPARATOR: &str = "--";
/// Codex 的 Stack 模型 id：`<prefix><key>/<model>`。
const CODEX_SEPARATOR: char = '/';
/// Claude Desktop 的 Stack 模型 id：`claude-fable-ccs-<6 位数字>`（[`desktop_code`]）。Desktop 的
/// 选择器只收含 `claude` 等字样、又不含厂商名（kimi、glm、seed- 等一长串黑名单）的 id，所以 id
/// 里放不下 key 和模型名，只放数字；末尾 8 位数字会被当成日期剥掉，所以是 6 位。`fable` 前缀让
/// Desktop 给出 effort 选择器，内嵌的 Claude Code 不会因此把它当成真的 Fable。
const DESKTOP_PREFIX: &str = "claude-fable-ccs-";
const DESKTOP_CODE_DIGITS: usize = 6;

/// key 的最大长度：只是为了模型 id 不至于太长。
const KEY_MAX_LEN: usize = 24;

/// Claude Code 在没有 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 时按这个窗口算。
pub const CLAUDE_DEFAULT_WINDOW: u64 = 200_000;

/// 支持 Stack 模型的应用。
pub fn supports_stack(app: &AppType) -> bool {
    matches!(
        app,
        AppType::Claude | AppType::Codex | AppType::ClaudeDesktop
    )
}

/// 给 `provider` 一个 key：登记过就用原来的（移除后重新加入，旧会话里的 id 继续有效），
/// 没有就按图标、名称生成一个新的写进登记簿。
///
/// 新 key 和登记簿里所有的 key 去重，不只是当前成员：已移除、已删除的供应商的 key 也
/// 占着位置，旧 id 才不会被发给新来的这家。自动分配的 key 保持小写（历史 key 全小写，
/// 视觉一致）；大写仅由用户手动改名引入。去重忽略大小写（`key_taken`）。
pub fn allocate_key(stack: &mut StackState, provider: &Provider) -> String {
    if let Some(key) = stack.key_of(&provider.id) {
        return key.to_string();
    }
    let lower_slug = |text: &str| slug(text).to_ascii_lowercase();
    // 自定义图标是一长串 base64，拿来生成 key 没有意义，只用内置图标名
    let icon = provider
        .icon
        .as_deref()
        .filter(|icon| !crate::services::provider::icon::is_custom_icon(icon));
    let base = [icon, Some(provider.name.as_str())]
        .into_iter()
        .flatten()
        .map(lower_slug)
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| {
            let id: String = lower_slug(&provider.id)
                .chars()
                .filter(|c| *c != '-')
                .take(6)
                .collect();
            format!("p{id}")
        });
    let mut key = base.clone();
    let mut suffix = 2;
    // fork: 墓碑 key 同样占位——旧 id 不能被发给新来的这家（与登记簿同一不变量）
    while key_taken(stack, &key) {
        key = format!("{base}-{suffix}");
        suffix += 1;
    }
    stack.keys.insert(key.clone(), provider.id.clone());
    key
}

/// fork: 校验并归一化用户自定义 key（doc/20261009-设计文档 §6、实施计划-聚合模式五项
/// 优化）：输入经 slug 规则归一化（`[a-zA-Z0-9-]`、连续横线合并、首尾横线去掉、超长
/// 截断），之后仍须非空、不撞保留字 `default`（忽略大小写——decode 端解绑判断本就
/// 忽略，精确匹配会让 `Default` 绕过校验却被代理当解绑 id）、不与登记簿冲突
/// （忽略大小写 + 豁免 `self_provider_id` 自己的条目：防模型列表里 `Zhipu`/`zhipu`
/// 视觉重复，同时允许成员把自己的 key 升级大小写）。
/// 墓碑**不挡**手动改名（doc/20261009-五项优化验收后收敛）：用户明确输入的名字
/// 由用户自负责，复用旧 key 的后果（旧会话请求转投这家）写进 UI hint；墓碑只在
/// `allocate_key` 自动分配时避让——自动起名没有用户认知机会，必须防旧 id 静默换主。
/// 返回归一化后的 key。
pub fn validate_member_key(
    stack: &StackState,
    raw: &str,
    self_provider_id: &str,
) -> Result<String, String> {
    let normalized = slug(raw);
    if normalized.is_empty() {
        return Err(format!(
            "分组 key「{raw}」归一化后为空，请使用字母/数字 (Key normalizes to empty; use letters or digits)"
        ));
    }
    if normalized.eq_ignore_ascii_case(crate::proxy::route_prefix::RESERVED_ROUTE_KEY) {
        return Err(
            "分组 key 不能是保留字 default（解绑语义）(Key \"default\" is reserved for unbinding)"
                .to_string(),
        );
    }
    if key_taken_except_self(stack, &normalized, self_provider_id) {
        return Err(format!(
            "分组 key「{normalized}」已被使用 (Key \"{normalized}\" is already taken)"
        ));
    }
    Ok(normalized)
}

/// key 是否已被占用（大小写不敏感，登记簿 + 墓碑全量）——**仅 `allocate_key` 自动
/// 分配用**：自动起名没有用户认知机会，必须避开一切历史 id（含墓碑大小写变体），
/// 防止旧会话带的 id 静默换主。
fn key_taken(stack: &StackState, key: &str) -> bool {
    stack
        .keys
        .keys()
        .chain(stack.reserved_keys.iter())
        .any(|existing| existing.eq_ignore_ascii_case(key))
}

/// 手动改名的冲突判定：只看登记簿，忽略大小写（防模型列表视觉重复）并豁免
/// `self_id` 自己的现 key（大小写重排不算冲突）。墓碑不参与（见
/// `validate_member_key` 的口径说明）。
fn key_taken_except_self(stack: &StackState, key: &str, self_id: &str) -> bool {
    stack
        .keys
        .iter()
        .any(|(k, v)| v != self_id && k.eq_ignore_ascii_case(key))
}

/// ASCII 字母数字（大小写均可），其余字符（含 `.`）换成 `-`，连续的 `-` 合并，首尾的
/// `-` 去掉。结果里不会有 `.`（Claude/Codex id 的 key↔模型分隔符）。
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= KEY_MAX_LEN {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Stack 模型 id。Claude 的上游是 1M 窗口时末尾带 `[1M]`，Claude Code 才按 1M 计算；Claude
/// Desktop 的 1M 写在模型条目里（`supports1m` + `prefer1m`），id 不带标记。
pub fn encode(prefix: &str, app: &AppType, key: &str, model: &str, one_m: bool) -> String {
    match app {
        AppType::Codex => format!("{prefix}{key}{CODEX_SEPARATOR}{model}"),
        AppType::ClaudeDesktop => format!("{DESKTOP_PREFIX}{}", desktop_code(key, model)),
        _ => {
            let marker = if one_m { ONE_M_MARKER_FOR_CLIENT } else { "" };
            format!("{prefix}{CLAUDE_INFIX}{key}{CLAUDE_SEPARATOR}{model}{marker}")
        }
    }
}

/// fork: 成员短形式 id（仅 key，选中即走该成员默认模型；保留 key `default` 为解绑语义）。
/// 两个应用的短形式同构，`app` 仅为兼容既有调用签名保留。
pub fn encode_short(prefix: &str, _app: &AppType, key: &str) -> String {
    format!("{prefix}{key}")
}

/// Claude Desktop Stack 模型 id 里的数字：key 和上游模型名的 FNV-1a 散列取 6 位。不用标准库的
/// 散列：它不保证跨版本稳定，客户端会一直带着选中过的 id。
fn desktop_code(key: &str, model: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes().chain([0]).chain(model.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!(
        "{:0width$}",
        hash % 10u64.pow(DESKTOP_CODE_DIGITS as u32),
        width = DESKTOP_CODE_DIGITS
    )
}

/// 解码的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded<'a> {
    /// 不带保留前缀：普通模型名，照旧走代理路由。
    Plain,
    /// 带保留前缀，但切不出 key 和模型。
    Malformed,
    /// 带保留前缀。`model` 可以含 `/` 和 `--`（key 里没有这两种分隔符，在第一个处切开）。
    Stack {
        key: &'a str,
        model: &'a str,
        /// Claude id 末尾带着 1M 标记。
        one_m: bool,
    },
    /// fork: 带保留前缀但没有分隔符——成员「默认模型」短形式 id（保留 key `default`
    /// 由代理层解绑语义处理，见 `route_prefix::apply_session_routing`）。
    Short { key: &'a str },
    /// Claude Desktop 的 Stack id：只有数字，key 和模型要按名单反查（[`resolve`]）。
    Code(&'a str),
}

/// 按客户端的格式解码模型 id。Claude：`<prefix>claude-` 开头；Codex：`<prefix>` 开头。
/// 带前缀但没有分隔符的是短形式（fork：Codex 旧行为是当普通模型名 Plain，已反转）。
/// Claude 的 id 末尾可带 `[1M]`。
pub fn decode<'a>(prefix: &str, app: &AppType, id: &'a str) -> Decoded<'a> {
    let (rest, separator, one_m) = match app {
        AppType::Claude => {
            let Some(rest) = id.strip_prefix(prefix) else {
                return Decoded::Plain;
            };
            // fork: 短形式 `<prefix><key>` 没有 `claude-` 中缀；完整形式剥掉中缀后按 `--` 切分。
            let Some(rest) = rest.strip_prefix(CLAUDE_INFIX) else {
                return if rest.is_empty() {
                    Decoded::Malformed
                } else {
                    Decoded::Short { key: rest }
                };
            };
            let stripped = strip_one_m_suffix_for_upstream(rest);
            (stripped, CLAUDE_SEPARATOR, stripped.len() != rest.len())
        }
        AppType::Codex => {
            let Some(rest) = id.strip_prefix(prefix) else {
                return Decoded::Plain;
            };
            (rest, "/", false)
        }
        AppType::ClaudeDesktop => {
            let Some(rest) = id.strip_prefix(DESKTOP_PREFIX) else {
                return Decoded::Plain;
            };
            // 1M 条目 Desktop 发请求时用 `<id>[1m]`（`prefer1m` 自动补；旧版另列一行）
            let code = strip_one_m_suffix_for_upstream(rest);
            return if code.len() == DESKTOP_CODE_DIGITS && code.bytes().all(|b| b.is_ascii_digit())
            {
                Decoded::Code(code)
            } else {
                Decoded::Malformed
            };
        }
        _ => return Decoded::Plain,
    };
    match rest.split_once(separator) {
        Some((key, model)) if !key.is_empty() && !model.is_empty() => {
            Decoded::Stack { key, model, one_m }
        }
        // fork: 无分隔符且 key 非空 → 短形式（成员默认模型）
        None if !rest.is_empty() => Decoded::Short { key: rest },
        _ => Decoded::Malformed,
    }
}

/// Claude Stack 供应商发布给客户端的一个模型（Codex 的由目录条目描述，见
/// `codex_config::plan_codex_stack_catalog`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackModel {
    /// 发布给客户端的 id（带保留前缀）。
    pub id: String,
    /// 发往上游的模型名：行里配置的原值（可能带 `[1M]`，转发时和路由请求一样处理）。
    pub upstream: String,
    /// 模型自己的显示名（行里配的，没有是模型名），不带供应商名。
    pub name: String,
    /// 选择器里显示的名字：`<显示名>（<供应商名>）`。
    pub display_name: String,
    /// 选择器里的说明。
    pub description: String,
    /// 上游是 1M 窗口（id 带 `[1M]`）。
    pub one_m: bool,
    /// 非 1M 模型的窗口：行里的 `CLAUDE_CODE_MAX_CONTEXT_TOKENS`，没有是 200K。
    pub window: u64,
}

/// Claude 行发布的模型：配了 Stack 模型列表（`meta.stackModels`）就是列表，清空了就什么都不
/// 发布；没配时是模型映射，即 `ANTHROPIC_MODEL` 和各档 `ANTHROPIC_DEFAULT_*_MODEL`，显示名取
/// 对应档位的 `*_MODEL_NAME`。按去掉 1M 标记后的名字去重（任何一处带标记就按 1M）。
pub fn claude_models(prefix: &str, key: &str, provider: &Provider) -> Vec<StackModel> {
    let listed = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.stack_models.as_deref());
    let found = match listed {
        Some(list) => listed_models(list),
        None => claude_env(provider).map(mapped_models).unwrap_or_default(),
    };
    stack_models(prefix, key, provider, found)
}

/// 行里要发布的一个模型（还没加前缀）。
struct Found {
    /// 去掉 1M 标记的模型名。
    model: String,
    /// 发往上游的原值（1M 模型带标记）。
    upstream: String,
    name: Option<String>,
    one_m: bool,
    /// 来自 meta.stackModels 行的上下文窗口（token 数）；映射来源和手动填入的行为 None。
    context_length: Option<u64>,
}

/// 按去掉 1M 标记后的名字去重地加入 `found`。同一个模型有一处带 1M 标记就按 1M，发往上游的
/// 也用带标记的那个写法；显示名取第一个有的；context_length 取第一个有的。
fn push_found(
    found: &mut Vec<Found>,
    upstream: &str,
    name: Option<String>,
    context_length: Option<u64>,
) {
    let model = strip_one_m_suffix_for_upstream(upstream).trim().to_string();
    if model.is_empty() {
        return;
    }
    let one_m = has_one_m_marker(upstream);
    match found.iter_mut().find(|entry| entry.model == model) {
        Some(entry) => {
            if one_m && !entry.one_m {
                entry.upstream = upstream.to_string();
                entry.one_m = true;
            }
            if entry.name.is_none() {
                entry.name = name;
            }
            if entry.context_length.is_none() {
                entry.context_length = context_length;
            }
        }
        None => found.push(Found {
            model,
            upstream: upstream.to_string(),
            name,
            one_m,
            context_length,
        }),
    }
}

fn claude_env(provider: &Provider) -> Option<&Map<String, Value>> {
    provider
        .settings_config
        .get("env")
        .and_then(Value::as_object)
}

/// Stack 模型列表（`meta.stackModels`）里的模型。
fn listed_models(list: &[ClaudeStackModel]) -> Vec<Found> {
    let mut found = Vec::new();
    for entry in list {
        let raw = entry.model.trim();
        let model = strip_one_m_suffix_for_upstream(raw).trim();
        let upstream = if entry.one_m || has_one_m_marker(raw) {
            format!("{model}{ONE_M_MARKER_FOR_CLIENT}")
        } else {
            model.to_string()
        };
        let name = entry
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        push_found(&mut found, &upstream, name, entry.context_length);
    }
    found
}

/// 模型映射里的模型：`ANTHROPIC_MODEL` 和各档 `ANTHROPIC_DEFAULT_*_MODEL`。
fn mapped_models(env: &Map<String, Value>) -> Vec<Found> {
    const ROLES: [(&str, Option<&str>); 5] = [
        ("ANTHROPIC_MODEL", None),
        (
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            Some("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            Some("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"),
        ),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            Some("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"),
        ),
        (
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            Some("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"),
        ),
    ];
    let mut found = Vec::new();
    for (model_key, name_key) in ROLES {
        let Some(upstream) = env_string(env, model_key) else {
            continue;
        };
        let name = name_key.and_then(|name_key| env_string(env, name_key).map(str::to_string));
        // 映射来源没有行级 context_length，使用 provider 级窗口兜底
        push_found(&mut found, upstream, name, None);
    }
    found
}

/// 给找到的模型加上前缀、显示名和窗口。窗口优先级：行级 context_length >
/// 供应商级 CLAUDE_CODE_MAX_CONTEXT_TOKENS > 200K 默认。1M 模型强制 1_000_000。
fn stack_models(
    prefix: &str,
    key: &str,
    provider: &Provider,
    found: Vec<Found>,
) -> Vec<StackModel> {
    let provider_window = claude_env(provider)
        .and_then(|env| env.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS"))
        .and_then(|value| match value {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        })
        .filter(|window| *window > 0)
        .unwrap_or(CLAUDE_DEFAULT_WINDOW);
    found
        .into_iter()
        .map(|found| {
            let name = found.name.unwrap_or_else(|| found.model.clone());
            // 1M 模型固定 1M 窗口；普通模型优先用行级值，退回 provider_window
            let shown_window = if found.one_m {
                1_000_000
            } else {
                found.context_length.unwrap_or(provider_window)
            };
            // `window` 字段记录非 1M 时实际生效的窗口，供路由决策使用
            let window = if found.one_m {
                provider_window
            } else {
                found.context_length.unwrap_or(provider_window)
            };
            StackModel {
                id: encode(prefix, &AppType::Claude, key, &found.model, found.one_m),
                display_name: display_name(&name, provider_label(provider)),
                name,
                description: model_description(&found.model, shown_window),
                upstream: found.upstream,
                one_m: found.one_m,
                window,
            }
        })
        .collect()
}

/// Codex 行发布的模型 id：行里的模型目录，没有配置目录时只有行的 `model`。显示名和窗口
/// 由目录条目决定（`codex_config::plan_codex_stack_catalog`）。
pub fn codex_model_ids(prefix: &str, key: &str, provider: &Provider) -> Vec<String> {
    let config = provider
        .settings_config
        .get("config")
        .and_then(Value::as_str)
        .unwrap_or("");
    crate::codex_config::codex_published_models(&provider.settings_config, config)
        .into_iter()
        .map(|model| encode(prefix, &AppType::Codex, key, &model, false))
        .collect()
}

/// Claude Desktop Stack 供应商发布的一个模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopStackModel {
    /// 发布给 Desktop 的 id（`claude-fable-ccs-<数字>`）。
    pub id: String,
    /// 发往上游的模型名：行里映射表的目标模型。
    pub upstream: String,
    /// 选择器里显示的名字（`labelOverride`）：`<供应商名> / <模型显示名>`。
    pub label: String,
    /// 映射表里这个模型标了 1M。
    pub supports_1m: bool,
}

/// Claude Desktop 行发布的模型：映射表的目标模型，按模型名去重（任何一条标了 1M 就按 1M），
/// 显示名取映射表里的，没有是模型名。映射表不完整（不能做路由）的行什么都不发布。
pub fn desktop_models(key: &str, provider: &Provider) -> Vec<DesktopStackModel> {
    let Ok(routes) = crate::claude_desktop_config::proxy_model_routes(provider) else {
        return Vec::new();
    };
    let mut models: Vec<DesktopStackModel> = Vec::new();
    for route in routes {
        if let Some(existing) = models
            .iter_mut()
            .find(|model| model.upstream == route.upstream_model)
        {
            existing.supports_1m |= route.supports_1m;
            continue;
        }
        let name = route
            .label_override
            .unwrap_or_else(|| route.upstream_model.clone());
        models.push(DesktopStackModel {
            id: encode("", &AppType::ClaudeDesktop, key, &route.upstream_model, false),
            label: format!("{} / {name}", provider.name),
            upstream: route.upstream_model,
            supports_1m: route.supports_1m,
        });
    }
    models
}

/// Claude Desktop 的这些成员发布给客户端的模型，按名单顺序。路由那家不发布：它的映射表就是
/// 默认的那几行。两个模型撞了同一个数字（概率很小）时后一个不发布：id 是散列出来的，换数字
/// 就不稳定了。
pub fn desktop_published(members: &[Member]) -> Vec<DesktopStackModel> {
    let mut seen = std::collections::HashSet::new();
    let mut published = Vec::new();
    for member in members.iter().filter(|member| !member.route) {
        for model in desktop_models(&member.key, &member.provider) {
            if !seen.insert(model.id.clone()) {
                log::warn!(
                    "Claude Desktop 聚合模型 {} 的 id {} 与前面的模型重复，不发布",
                    model.upstream,
                    model.id
                );
                continue;
            }
            published.push(model);
        }
    }
    published
}

/// 选择器里 Stack 模型的显示名：`<模型显示名>（<供应商名>）`。
pub fn display_name(model: &str, provider_name: &str) -> String {
    format!("{model}({provider_name})")
}

/// 聚合显示名里供应商的叫法：简称（`meta.shortName`，trim 后非空）优先，回退 `name`。
pub fn provider_label(provider: &Provider) -> &str {
    provider
        .meta
        .as_ref()
        .and_then(|meta| meta.short_name.as_deref())
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(&provider.name)
}

/// 选择器里 Stack 模型的说明：`<上游模型名> · <窗口>`。显示名可以是用户自己起的，id 又带着
/// 前缀，上游真正的模型名只有这里看得到；供应商名已经在显示名里，不再重复。窗口未知
/// （`0`）时只写模型名。
pub fn model_description(model: &str, window: u64) -> String {
    match window_label(window) {
        Some(window) => format!("{model} · {window}"),
        None => model.to_string(),
    }
}

/// 窗口的简写：整百万写 `1M`，其余按千写 `256K`，不足一千照原样。
fn window_label(window: u64) -> Option<String> {
    match window {
        0 => None,
        w if w % 1_000_000 == 0 => Some(format!("{}M", w / 1_000_000)),
        w if w >= 1_000 => Some(format!("{}K", (w + 500) / 1_000)),
        w => Some(w.to_string()),
    }
}

/// 一家 Stack 供应商发布给客户端的模型 id。Codex 路由那家的整张目录就是默认路由的目录行，
/// 不再带前缀发布；Claude 路由那家整张列表照常发布（它的第一个模型同时占着四档别名，见
/// [`claude_route_default`]）。
fn model_ids_of(
    prefix: &str,
    app: &AppType,
    key: &str,
    provider: &Provider,
    route: bool,
) -> Vec<String> {
    match app {
        AppType::Claude => claude_models(prefix, key, provider)
            .into_iter()
            .map(|model| model.id)
            .collect(),
        AppType::Codex if !route => codex_model_ids(prefix, key, provider),
        AppType::ClaudeDesktop if !route => desktop_models(key, provider)
            .into_iter()
            .map(|model| model.id)
            .collect(),
        _ => Vec::new(),
    }
}

/// 名单里的一家。
#[derive(Debug, Clone)]
pub struct Member {
    pub provider: Provider,
    pub key: String,
    /// 这家是路由那家（默认），见 [`is_published`]。
    pub route: bool,
    /// 发布给客户端的模型 id。
    pub model_ids: Vec<String>,
}

/// 名单里还在库里的成员，按加入顺序。库里已经没有的跳过（删除供应商会先把它移出名单，
/// 删行前失败才会留下）。`route` 是代理模式下的路由供应商（不在代理模式时为 `None`）。
pub fn members(
    prefix: &str,
    db: &Database,
    app: &AppType,
    stack: &StackState,
    route: Option<&str>,
) -> Result<Vec<Member>, AppError> {
    let mut members = Vec::with_capacity(stack.members.len());
    for id in &stack.members {
        let Some(key) = stack.key_of(id) else {
            log::warn!("{} 的 Stack 模型成员 {id} 没有登记 key，跳过", app.as_str());
            continue;
        };
        let Some(provider) = db.get_provider_by_id(id, app.as_str())? else {
            continue;
        };
        let route = route == Some(id.as_str());
        let model_ids = model_ids_of(prefix, app, key, &provider, route);
        members.push(Member {
            key: key.to_string(),
            provider,
            route,
            model_ids,
        });
    }
    Ok(members)
}

/// 这个成员发布 Stack 模型。Codex 路由那家不发布（它的模型已经是默认路由的目录行），Claude
/// 路由那家照常发布；名单都保留。契约、Codex 目录、Claude Code 的模型发现和给前端的名单都按
/// 这一条算。
pub fn is_published(member: &Member) -> bool {
    !member.route || !member.model_ids.is_empty()
}

/// 发布 Stack 模型的成员（按名单顺序，见 [`is_published`]）。Stack 模式关着（路由模式）时
/// 没有：名单留着，下次进入 Stack 模式时恢复。
pub fn published_members(
    prefix: &str,
    db: &Database,
    app: &AppType,
    stack: &StackState,
    route: Option<&str>,
) -> Result<Vec<Member>, AppError> {
    if !stack.enabled || stack.members.is_empty() || !supports_stack(app) {
        return Ok(Vec::new());
    }
    let mut members = members(prefix, db, app, stack, route)?;
    members.retain(is_published);
    Ok(members)
}

/// Claude 的这些成员发布给客户端的模型，按名单顺序。
pub fn claude_published(prefix: &str, members: &[Member]) -> Vec<StackModel> {
    members
        .iter()
        .flat_map(|member| claude_models(prefix, &member.key, &member.provider))
        .collect()
}

/// 默认那家（路由）列表里的第一个模型：Stack 模式下 Claude Code 的四档别名（启动默认、后台
/// 任务、子代理别名）都指向它。列表的顺序就是模型映射的顺序（`ANTHROPIC_MODEL` 在前），
/// 所以没配列表的行用的是它的主模型。路由那家不在发布的成员里（Stack 模式关着、它没有模型）
/// 时没有。
pub fn claude_route_default(prefix: &str, members: &[Member]) -> Option<StackModel> {
    let route = members.iter().find(|member| member.route)?;
    claude_models(prefix, &route.key, &route.provider)
        .into_iter()
        .next()
}

/// 聚合模式默认分组四档别名各指向的模型（doc/20261015-设计文档-聚合默认分组模型映射）。
#[derive(Debug, Clone, PartialEq)]
pub struct RouteRoles {
    pub haiku: StackModel,
    pub sonnet: StackModel,
    pub opus: StackModel,
    pub fable: StackModel,
}

/// 四档别名指向的模型：每档读默认分组 `meta.stackRoleMapping` 里配的模型名，在它自己发布
/// 的模型列表里按去掉 `[1M]` 标记后的名字查表（id 的 1M 跟随列表行）；未配或不在列表里
/// 回落列表第一模型（默认模型，即 [`claude_route_default`]）。没有 route 成员（或它没有可
/// 发布的模型）时为 `None`，与 `claude_route_default` 一致。
///
/// 映射只认 `meta.stackRoleMapping` 这份显式数据，不读 env 档位键：`ProviderService` 保存时
/// 会把缺失的档位键自动回填成 `ANTHROPIC_MODEL`（`normalize_claude_models_in_value`），若把
/// env 当映射源，回填的旧默认值会静默生效、压过「第一模型占四档」的语义。
pub fn claude_route_roles(prefix: &str, members: &[Member]) -> Option<RouteRoles> {
    let fallback = claude_route_default(prefix, members)?;
    let route = members.iter().find(|member| member.route)?;
    let list = claude_models(prefix, &route.key, &route.provider);
    let mapping = route
        .provider
        .meta
        .as_ref()
        .and_then(|meta| meta.stack_role_mapping.as_ref());
    let tier = |mapped: Option<&str>| -> StackModel {
        let hit = mapped
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| strip_one_m_suffix_for_upstream(value).trim().to_string())
            .and_then(|model| {
                list.iter()
                    .find(|entry| {
                        strip_one_m_suffix_for_upstream(&entry.upstream).trim() == model
                    })
                    .cloned()
            });
        hit.unwrap_or_else(|| fallback.clone())
    };
    Some(RouteRoles {
        haiku: tier(mapping.and_then(|mapping| mapping.haiku.as_deref())),
        sonnet: tier(mapping.and_then(|mapping| mapping.sonnet.as_deref())),
        opus: tier(mapping.and_then(|mapping| mapping.opus.as_deref())),
        fable: tier(mapping.and_then(|mapping| mapping.fable.as_deref())),
    })
}

/// 这个应用在 Stack 模式（代理模式且 Stack 模式开着）。读不出状态按不在处理。
pub fn stack_mode_now(app: &AppType) -> bool {
    if !supports_stack(app) {
        return false;
    }
    state::stack_mode(&DeviceStore::for_device(), app.as_str()).unwrap_or_else(|error| {
        log::warn!(
            "读取 {} 的 Stack 模式失败，按不在处理: {error}",
            app.as_str()
        );
        false
    })
}

/// 在 Stack 名单里（不管什么模式）。
pub fn is_member(app: &AppType, provider_id: &str) -> Result<bool, AppError> {
    if !supports_stack(app) {
        return Ok(false);
    }
    Ok(state::stack(&DeviceStore::for_device(), app.as_str())?.is_member(provider_id))
}

/// Claude Code 现在发布的 Stack 模型：代理模式下按已落定的名单和路由算，不在代理模式时没有。
pub fn claude_published_now(prefix: &str, db: &Database) -> Result<Vec<StackModel>, AppError> {
    let app = AppType::Claude;
    let store = DeviceStore::for_device();
    let mode = state::mode_state(&store, app.as_str())?;
    if !mode.is_proxy() {
        return Ok(Vec::new());
    }
    let stack = state::stack(&store, app.as_str())?;
    Ok(claude_published(
        prefix,
        &published_members(prefix, db, &app, &stack, mode.proxy_route.as_deref())?,
    ))
}

/// Claude Desktop 现在发布的 Stack 模型（`/v1/models` 用）：同 [`claude_published_now`]。
pub fn desktop_published_now(db: &Database) -> Result<Vec<DesktopStackModel>, AppError> {
    let app = AppType::ClaudeDesktop;
    let store = DeviceStore::for_device();
    let mode = state::mode_state(&store, app.as_str())?;
    if !mode.is_proxy() {
        return Ok(Vec::new());
    }
    let stack = state::stack(&store, app.as_str())?;
    Ok(desktop_published(&published_members(
        &crate::settings::get_route_prefix(),
        db,
        &app,
        &stack,
        mode.proxy_route.as_deref(),
    )?))
}

/// 选中 Stack 模型的请求要发往的那一家。
#[derive(Debug, Clone)]
pub struct StackTarget {
    pub provider: Provider,
    /// 发往上游的模型名。
    pub upstream_model: String,
    /// 客户端发来的带前缀 id（只用于日志展示）。
    pub original_model: String,
}

/// 带保留前缀的 id 为什么解不出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackMiss {
    /// 这个应用不在 Stack 模式（路由模式下名单留着，但不发布、也不转发）。
    StackOff,
    /// key 没登记，或 id 切不出 key 和模型。
    Unknown,
    /// 这家已经从 Stack 名单移除。
    Removed,
    /// 这家已经从 CC Switch 删除。
    Deleted,
    /// fork: 短形式命中的成员没有默认模型（claude 无 stackModels/模型映射、codex 无 model）。
    NoDefaultModel,
}

impl StackMiss {
    /// 返回给客户端的错误文案。
    pub fn message(&self, model: &str) -> String {
        match self {
            Self::StackOff => format!(
                "聚合的模型 {model} 只能在聚合模式下使用，当前没有开启聚合模式，请在模型列表里重新选择 (Aggregated model {model} only works in Stack mode, which is off; pick a model from the model list again)"
            ),
            Self::Unknown => format!(
                "聚合的模型 {model} 在 CC Switch 里不存在，请在模型列表里重新选择 (Aggregated model {model} is unknown to CC Switch; pick a model from the model list again)"
            ),
            Self::Removed => format!(
                "聚合的模型 {model} 已从 CC Switch 移除，请在模型列表里重新选择 (Aggregated model {model} was removed from CC Switch; pick a model from the model list again)"
            ),
            Self::Deleted => format!(
                "聚合的模型 {model} 对应的供应商已删除，请在模型列表里重新选择 (The provider of aggregated model {model} was deleted; pick a model from the model list again)"
            ),
            Self::NoDefaultModel => format!(
                "聚合的模型 {model} 对应的成员没有可用的默认模型，请在该供应商的聚合模型列表里配置 (The aggregated member behind {model} has no default model; configure its stack model list)"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Resolved {
    /// 普通模型名，照旧走代理路由。
    Plain,
    Hit(Box<StackTarget>),
    /// 带保留前缀但解不出来：报错，不回落到默认路由。名单为空时也一样。
    Miss(StackMiss),
}

// fork: 以下三个 helper 供 resolve 与会话粘性层（route_prefix::apply_session_routing）共用
// （doc/20261009-设计文档-会话路由改造为聚合模式）。

/// key → 成员查表：Ok(Ok(provider)) 命中；Ok(Err(miss)) 是 Unknown/Removed/Deleted。
pub fn resolve_member(
    db: &Database,
    stack: &StackState,
    app: &AppType,
    key: &str,
) -> Result<Result<Provider, StackMiss>, AppError> {
    let Some(provider_id) = stack.keys.get(key) else {
        return Ok(Err(StackMiss::Unknown));
    };
    let provider = db.get_provider_by_id(provider_id, app.as_str())?;
    let Some(provider) = provider else {
        return Ok(Err(StackMiss::Deleted));
    };
    if !stack.is_member(provider_id) {
        return Ok(Err(StackMiss::Removed));
    }
    Ok(Ok(provider))
}

/// 成员的默认模型（发往上游的原值）：Claude 是发布列表第一个（stackModels[0]，
/// 未配列表时为模型映射默认 ANTHROPIC_MODEL）；Codex 复用 `model` / config TOML
/// `model =` 的既有解析（`codex_provider_upstream_model`，含 trim 与空值过滤）。
pub fn member_default_model(app: &AppType, key: &str, provider: &Provider) -> Option<String> {
    match app {
        AppType::Claude => {
            // 前缀只影响发布 id，不影响 upstream；传上游默认值即可
            claude_models("ccs-", key, provider)
                .first()
                .map(|model| model.upstream.clone())
        }
        _ => crate::proxy::providers::codex_provider_upstream_model(provider),
    }
}

/// 当前成员的 key 列表（fail-closed 报错文案用）。
pub fn available_keys(stack: &StackState) -> Vec<String> {
    stack
        .members
        .iter()
        .filter_map(|id| stack.key_of(id))
        .map(str::to_string)
        .collect()
}

/// 解析请求里的模型 id。不带保留前缀时不读任何状态，路由请求的路径不变。带前缀的只在
/// Stack 模式下解析：路由模式下名单留着，客户端手里旧的 Stack id 也不能转给名单里的那家。
pub fn resolve(
    prefix: &str,
    db: &Database,
    store: &DeviceStore,
    app: &AppType,
    model: &str,
) -> Result<Resolved, AppError> {
    let (key, model_part, one_m) = match decode(prefix, app, model) {
        Decoded::Plain => return Ok(Resolved::Plain),
        Decoded::Malformed => return Ok(Resolved::Miss(StackMiss::Unknown)),
        Decoded::Code(code) => return resolve_desktop(db, store, app, model, code),
        Decoded::Stack { key, model, one_m } => (key, model, one_m),
        // fork: 短形式（仅 key）→ 该成员的默认模型；保留 key `default` 的解绑语义
        // 在代理层（route_prefix::apply_session_routing），这里按不存在报错钉住边界。
        Decoded::Short { key } => {
            if key.eq_ignore_ascii_case(crate::proxy::route_prefix::RESERVED_ROUTE_KEY) {
                return Ok(Resolved::Miss(StackMiss::Unknown));
            }
            if !state::stack_mode(store, app.as_str())? {
                return Ok(Resolved::Miss(StackMiss::StackOff));
            }
            let stack = state::stack(store, app.as_str())?;
            let provider = match resolve_member(db, &stack, app, key)? {
                Ok(provider) => provider,
                Err(miss) => return Ok(Resolved::Miss(miss)),
            };
            let Some(upstream_model) = member_default_model(app, key, &provider) else {
                return Ok(Resolved::Miss(StackMiss::NoDefaultModel));
            };
            return Ok(Resolved::Hit(Box::new(StackTarget {
                provider,
                upstream_model,
                original_model: model.to_string(),
            })));
        }
    };
    if !state::stack_mode(store, app.as_str())? {
        return Ok(Resolved::Miss(StackMiss::StackOff));
    }
    let stack = state::stack(store, app.as_str())?;
    let provider = match resolve_member(db, &stack, app, key)? {
        Ok(provider) => provider,
        Err(miss) => return Ok(Resolved::Miss(miss)),
    };
    // Claude 发往上游的是行里配置的原值（可能带 1M 标记），和路由请求映射出来的一样；
    // 行里已经没有这个模型时照原样发（上游自己决定认不认）。Codex 的 id 就是行里的模型名。
    let upstream_model = match app {
        AppType::Claude => claude_models(prefix, key, &provider)
            .into_iter()
            .find(|published| {
                strip_one_m_suffix_for_upstream(&published.upstream).trim() == model_part
            })
            .map(|published| published.upstream),
        _ => None,
    }
    .unwrap_or_else(|| {
        if one_m {
            format!("{model_part}{ONE_M_MARKER_FOR_CLIENT}")
        } else {
            model_part.to_string()
        }
    });
    Ok(Resolved::Hit(Box::new(StackTarget {
        provider,
        upstream_model,
        original_model: model.to_string(),
    })))
}

/// Claude Desktop 的数字 id：先按名单顺序在发布出去的成员（路由那家以外）里找，和
/// [`desktop_published`] 的去重同一个顺序，撞号时命中的就是选择器里显示的那个；再找路由那家
/// （它变成默认后不再发布，但客户端可能还带着旧 id）。找不到再看登记簿里已移除的那些，区分
/// 「已移除」和「不存在」；已删除的供应商算不出模型，归到「不存在」。
fn resolve_desktop(
    db: &Database,
    store: &DeviceStore,
    app: &AppType,
    model: &str,
    code: &str,
) -> Result<Resolved, AppError> {
    if !state::stack_mode(store, app.as_str())? {
        return Ok(Resolved::Miss(StackMiss::StackOff));
    }
    let stack = state::stack(store, app.as_str())?;
    let route = state::mode_state(store, app.as_str())?.proxy_route;
    let id = format!("{DESKTOP_PREFIX}{code}");
    let members = members(&crate::settings::get_route_prefix(), db, app, &stack, route.as_deref())?;
    let published = members.iter().filter(|member| !member.route);
    let route = members.iter().filter(|member| member.route);
    for member in published.chain(route) {
        if let Some(found) = desktop_models(&member.key, &member.provider)
            .into_iter()
            .find(|published| published.id == id)
        {
            return Ok(Resolved::Hit(Box::new(StackTarget {
                provider: member.provider.clone(),
                upstream_model: found.upstream,
                original_model: model.to_string(),
            })));
        }
    }
    let find = |key: &str, provider_id: &str| -> Result<bool, AppError> {
        let Some(provider) = db.get_provider_by_id(provider_id, app.as_str())? else {
            return Ok(false);
        };
        Ok(desktop_models(key, &provider)
            .iter()
            .any(|published| published.id == id))
    };
    for (key, provider_id) in &stack.keys {
        if !stack.is_member(provider_id) && find(key, provider_id)? {
            return Ok(Resolved::Miss(StackMiss::Removed));
        }
    }
    Ok(Resolved::Miss(StackMiss::Unknown))
}

/// 给前端：名单里的一家。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackMemberView {
    pub provider_id: String,
    /// fork Task 9: 分组 key（聚合视图展示与编辑用）。
    pub key: String,
    /// 发布给客户端的模型 id。
    pub model_ids: Vec<String>,
    /// 这家是默认那家（代理路由）。Claude 的照常发布，第一个模型同时占着四档别名；Codex 的
    /// 模型是默认路由的目录行，`model_ids` 为空，默认换到别家后才带前缀发布。
    pub route: bool,
    /// 这家不做默认、只做成员时发布几个模型。换默认的预览用它：现在的默认那家（Codex、
    /// Claude Desktop）`model_ids` 为空，换走后它的模型才发布。
    pub member_models: usize,
}

/// 给前端：Codex 聚合面板「子代理模型」下拉的一项（见 `codex_direct::stack_subagent_choices`）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackSubagentModel {
    /// 发布给客户端的模型 id（`ccs-<key>/<model>`），也是写进 `[agents]` 的值。
    pub id: String,
    pub provider_id: String,
    /// `<模型显示名>（<供应商名>）`，和模型选择器里一样。
    pub display_name: String,
    /// 这个模型支持的推理强度，按目录里的顺序。
    pub efforts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

/// 给前端：Stack 模式的状态、名单和提示。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackView {
    /// 在 Stack 模式（代理模式且 Stack 模式开着）。
    pub active: bool,
    pub members: Vec<StackMemberView>,
    /// Codex Stack 模型客户端看不到或看不全：`routeOwnsCatalog` 路由那家自己管理模型目录
    /// 文件，Stack 模型不发布；官方做路由时官方模型列表暂未取到：`officialModelsBundled`
    /// 暂用 Codex 自带的列表（可能缺账号专属的模型），`officialModelsUnavailable` Stack 模型
    /// 暂不可用；`officialModelsOutdated` 本机 Codex 太旧，拉到的官方列表里没有能选的模型。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<&'static str>,
    /// `notice` 是 `officialModelsBundled` / `officialModelsUnavailable` 时，没取到列表的原因
    /// 码：先是官方列表的（`noLogin`、`loginStale`、`loginIncomplete`、`noVersion`、
    /// `fetchFailed`、`invalidList`），不可用时再跟自带列表的（`noCli`、`cliFailed`、
    /// `nothingUsable`）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notice_reasons: Vec<&'static str>,
    /// Codex 客户端还在用旧的模型列表（启动时读的目录），Stack 模型看不到，要重启才行。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_clients: Option<StaleClients>,
    /// `stale_clients` 对应的目录和登录（见 `codex_client_catalog::revision`）：用户关掉提示后，
    /// 同一个供应商上目录又改了，这个值变了，前端重新显示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_revision: Option<String>,
    /// Codex：聚合面板「子代理模型」下拉的选项，发布 Stack 模型的成员按名单顺序。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subagent_models: Vec<StackSubagentModel>,
}

pub fn member_views(app: &AppType, members: &[Member]) -> Vec<StackMemberView> {
    members
        .iter()
        .map(|member| StackMemberView {
            provider_id: member.provider.id.clone(),
            key: member.key.clone(),
            model_ids: member.model_ids.clone(),
            route: member.route,
            member_models: if member.route {
                model_ids_of(&crate::settings::get_route_prefix(), app, &member.key, &member.provider, false).len()
            } else {
                member.model_ids.len()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(id: &str, name: &str, icon: Option<&str>, env: Value) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            name.to_string(),
            json!({ "env": env }),
            None,
        );
        provider.icon = icon.map(str::to_string);
        provider
    }

    #[test]
    fn keys_come_from_the_icon_then_the_name() {
        let mut stack = StackState::default();
        let kimi = provider("a", "Kimi For Coding", Some("kimi"), json!({}));
        let named = provider("b", "My Relay 2.0!", None, json!({}));
        let chinese = provider("c1b2c3d4-e5f6", "智谱", None, json!({}));
        let blank_icon = provider("d", "DeepSeek", Some("  "), json!({}));
        let custom_icon = provider(
            "e",
            "Acme",
            Some("data:image/png;base64,iVBORw0KGgo="),
            json!({}),
        );

        assert_eq!(allocate_key(&mut stack, &kimi), "kimi");
        assert_eq!(allocate_key(&mut stack, &named), "my-relay-2-0");
        assert_eq!(allocate_key(&mut stack, &chinese), "pc1b2c3");
        assert_eq!(allocate_key(&mut stack, &blank_icon), "deepseek");
        assert_eq!(allocate_key(&mut stack, &custom_icon), "acme");
    }

    #[test]
    fn keys_never_contain_the_separators_and_stay_short() {
        for text in ["a--b", "a/b", "--edge--", "UPPER__lower", &"x".repeat(80)] {
            let key = slug(text);
            assert!(!key.contains("--") && !key.contains('/'), "{text} → {key}");
            assert!(
                !key.starts_with('-') && !key.ends_with('-'),
                "{text} → {key}"
            );
            assert!(key.len() <= KEY_MAX_LEN, "{text} → {key}");
        }
        // 大小写保留；`.`（id 的 key↔模型分隔符）归一化为横线
        assert_eq!(slug("UPPER__lower"), "UPPER-lower");
        assert_eq!(slug("my.key"), "my-key");
    }

    #[test]
    fn a_key_stays_with_its_provider_forever() {
        let mut stack = StackState::default();
        let a = provider("a", "Kimi", Some("kimi"), json!({}));
        let b = provider("b", "Kimi Coding Plan", Some("kimi"), json!({}));

        assert_eq!(allocate_key(&mut stack, &a), "kimi");
        stack.members.push("a".to_string());
        // A 移除（登记簿保留）后，同图标的 B 拿不到 A 的 key。
        stack.members.clear();
        assert_eq!(allocate_key(&mut stack, &b), "kimi-2");
        // A 重新加入，拿回原来的 key。
        assert_eq!(allocate_key(&mut stack, &a), "kimi");
        assert_eq!(stack.keys.len(), 2);
    }

    #[test]
    fn claude_ids_round_trip() {
        let claude = AppType::Claude;
        for (model, one_m) in [
            ("kimi-k3", false),
            ("glm-5.2", true),
            ("vendor/model", false),
            ("model--with--dashes", true),
        ] {
            let id = encode("ccs-", &claude, "kimi", model, one_m);
            assert_eq!(
                decode("ccs-", &claude, &id),
                Decoded::Stack {
                    key: "kimi",
                    model,
                    one_m
                },
                "{id}"
            );
        }
        // 标记大小写不敏感。
        assert_eq!(
            decode("ccs-", &claude, "ccs-claude-k--m[1m]"),
            Decoded::Stack {
                key: "k",
                model: "m",
                one_m: true
            }
        );
    }

    #[test]
    fn codex_ids_round_trip() {
        let codex = AppType::Codex;
        let id = encode(
            "ccs-",
            &codex,
            "deepseek",
            "deepseek/deepseek-v4-pro",
            false,
        );
        assert_eq!(id, "ccs-deepseek/deepseek/deepseek-v4-pro");
        assert_eq!(
            decode("ccs-", &codex, &id),
            Decoded::Stack {
                key: "deepseek",
                model: "deepseek/deepseek-v4-pro",
                one_m: false
            }
        );
    }

    #[test]
    fn plain_ids_and_other_apps_are_not_decoded() {
        let (claude, codex) = (AppType::Claude, AppType::Codex);
        assert_eq!(decode("ccs-", &claude, "claude-sonnet-5"), Decoded::Plain);
        assert_eq!(decode("ccs-", &claude, "kimi-k3"), Decoded::Plain);
        // 路由那家自己的 `deepseek/…` 不被同名的 Stack key 截走。
        assert_eq!(
            decode("ccs-", &codex, "deepseek/deepseek-v4-pro"),
            Decoded::Plain
        );
        assert_eq!(
            decode("ccs-", &AppType::ClaudeDesktop, "ccs-k.m"),
            Decoded::Plain
        );
        assert_eq!(
            decode("ccs-", &AppType::GrokBuild, "ccs-k.m"),
            Decoded::Plain
        );
    }

    #[test]
    fn reserved_ids_that_do_not_split_are_malformed() {
        let (claude, codex) = (AppType::Claude, AppType::Codex);
        // 带前缀但切不出 key/model → Malformed（fork：无分隔符的非空 id 是短形式，不是 Malformed）
        assert_eq!(decode("ccs-", &claude, "ccs-claude---m"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &claude, "ccs-claude-k--"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &codex, "ccs-/m"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &codex, "ccs-k/"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &codex, "ccs-"), Decoded::Malformed);
    }

    #[test]
    fn claude_rows_publish_each_model_once() {
        let row = provider(
            "p",
            "Zhipu",
            None,
            json!({
                "ANTHROPIC_MODEL": "glm-5.2",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5.2[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "GLM 5.2",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "glm-5.2",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "glm-4.7-air",
                "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "128000"
            }),
        );
        let models = claude_models("ccs-", "zhipu", &row);
        assert_eq!(
            models,
            vec![
                StackModel {
                    id: "ccs-claude-zhipu--glm-5.2[1M]".to_string(),
                    upstream: "glm-5.2[1M]".to_string(),
                    name: "GLM 5.2".to_string(),
                    display_name: "GLM 5.2(Zhipu)".to_string(),
                    description: "glm-5.2 · 1M".to_string(),
                    one_m: true,
                    window: 128_000,
                },
                StackModel {
                    id: "ccs-claude-zhipu--glm-4.7-air".to_string(),
                    upstream: "glm-4.7-air".to_string(),
                    name: "glm-4.7-air".to_string(),
                    display_name: "glm-4.7-air(Zhipu)".to_string(),
                    description: "glm-4.7-air · 128K".to_string(),
                    one_m: false,
                    window: 128_000,
                },
            ]
        );
        let bare = provider("q", "Bare", None, json!({ "ANTHROPIC_AUTH_TOKEN": "sk" }));
        assert!(claude_models("ccs-", "bare", &bare).is_empty());
        let default_window = provider("r", "R", None, json!({ "ANTHROPIC_MODEL": "m" }));
        assert_eq!(
            claude_models("ccs-", "r", &default_window)[0].window,
            CLAUDE_DEFAULT_WINDOW
        );
    }

    #[test]
    fn the_description_names_the_upstream_model_and_its_window() {
        assert_eq!(model_description("kimi-k3", 256_000), "kimi-k3 · 256K");
        assert_eq!(model_description("kimi-k3", 262_144), "kimi-k3 · 262K");
        assert_eq!(model_description("glm-5.2", 1_000_000), "glm-5.2 · 1M");
        assert_eq!(model_description("glm-5.2", 1_050_000), "glm-5.2 · 1050K");
        assert_eq!(model_description("m", 0), "m");
    }

    fn with_stack_models(mut provider: Provider, models: Value) -> Provider {
        provider.meta = Some(crate::provider::ProviderMeta {
            stack_models: serde_json::from_value(models).unwrap(),
            ..Default::default()
        });
        provider
    }

    fn with_role_mapping(mut provider: Provider, mapping: Value) -> Provider {
        provider
            .meta
            .get_or_insert_with(Default::default)
            .stack_role_mapping = serde_json::from_value(mapping).unwrap();
        provider
    }

    #[test]
    fn an_emptied_list_publishes_nothing_and_an_unset_one_follows_the_mapping() {
        let mapped = provider("p", "Kimi", None, json!({ "ANTHROPIC_MODEL": "kimi-k3" }));
        let emptied = with_stack_models(mapped.clone(), json!([]));
        assert!(claude_models("ccs-", "kimi", &emptied).is_empty());
        assert_eq!(
            serde_json::to_value(emptied.meta.as_ref().unwrap()).unwrap()["stackModels"],
            json!([])
        );

        let unset = with_stack_models(mapped, Value::Null);
        assert_eq!(unset.meta.as_ref().unwrap().stack_models, None);
        let ids: Vec<String> = claude_models("ccs-", "kimi", &unset)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(ids, vec!["ccs-claude-kimi--kimi-k3"]);
        let meta = serde_json::to_value(unset.meta.as_ref().unwrap()).unwrap();
        assert!(meta.get("stackModels").is_none());
    }

    #[test]
    fn a_stack_model_list_replaces_the_mapping() {
        let row = with_stack_models(
            provider(
                "p",
                "Kimi",
                None,
                json!({
                    "ANTHROPIC_MODEL": "kimi-k3",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "kimi-k3-turbo",
                    "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "256000"
                }),
            ),
            json!([
                { "model": " kimi-k3 ", "displayName": "Kimi K3" },
                { "model": "kimi-k3-thinking", "displayName": " ", "oneM": true },
                { "model": "  " },
                { "model": "kimi-k3[1m]", "displayName": "ignored" }
            ]),
        );
        let summary: Vec<(String, String, String, bool, u64)> = claude_models("ccs-", "kimi", &row)
            .into_iter()
            .map(|model| {
                (
                    model.id,
                    model.upstream,
                    model.display_name,
                    model.one_m,
                    model.window,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "ccs-claude-kimi--kimi-k3[1M]".to_string(),
                    "kimi-k3[1M]".to_string(),
                    "Kimi K3(Kimi)".to_string(),
                    true,
                    256_000,
                ),
                (
                    "ccs-claude-kimi--kimi-k3-thinking[1M]".to_string(),
                    "kimi-k3-thinking[1M]".to_string(),
                    "kimi-k3-thinking(Kimi)".to_string(),
                    true,
                    256_000,
                ),
            ]
        );
    }

    #[test]
    fn context_length_from_stack_model_row_beats_provider_window() {
        // 行级 contextLength 优先于供应商级 CLAUDE_CODE_MAX_CONTEXT_TOKENS；
        // 无行级值时退回供应商窗口。
        let row = with_stack_models(
            provider(
                "p",
                "Kimi",
                None,
                json!({ "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "256000" }),
            ),
            json!([
                { "model": "kimi-k3", "contextLength": 128000 },
                { "model": "kimi-k4" }
            ]),
        );
        let models = claude_models("ccs-", "kimi", &row);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].window, 128_000, "行级 contextLength 应覆盖供应商窗口");
        assert_eq!(models[0].description, "kimi-k3 · 128K");
        assert_eq!(models[1].window, 256_000, "无行级值时应退回供应商窗口");
        assert_eq!(models[1].description, "kimi-k4 · 256K");
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: DeviceStore,
        db: Database,
    }

    /// Claude Code、Codex 都在 Stack 模式。Claude 的 kimi、zhipu 在名单里；gone 登记过但已
    /// 移除；deleted 在名单里但行已经删了。Codex 名单为空。
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = DeviceStore::at(dir.path());
        let db = Database::memory().unwrap();
        for row in [
            provider(
                "kimi",
                "Kimi",
                Some("kimi"),
                json!({ "ANTHROPIC_MODEL": "kimi-k3" }),
            ),
            provider(
                "zhipu",
                "Zhipu",
                None,
                json!({ "ANTHROPIC_MODEL": "glm-5.2[1M]" }),
            ),
            provider("gone", "Gone", None, json!({ "ANTHROPIC_MODEL": "g-1" })),
        ] {
            db.save_provider("claude", &row).unwrap();
        }
        state::update(&store, |live| {
            let claude = live.apps.entry("claude".to_string()).or_default();
            claude.mode = Some(state::Mode::Proxy);
            let stack = &mut claude.stack;
            stack.enabled = true;
            stack.members = ["kimi", "zhipu", "deleted"].map(str::to_string).to_vec();
            for id in ["kimi", "zhipu", "gone", "deleted"] {
                stack.keys.insert(id.to_string(), id.to_string());
            }
            let codex = live.apps.entry("codex".to_string()).or_default();
            codex.mode = Some(state::Mode::Proxy);
            codex.stack.enabled = true;
        })
        .unwrap();
        Fixture {
            _dir: dir,
            store,
            db,
        }
    }

    fn resolve_in(fx: &Fixture, app: AppType, model: &str) -> Resolved {
        resolve("ccs-", &fx.db, &fx.store, &app, model).unwrap()
    }

    fn hit(resolved: Resolved) -> (String, String, String) {
        match resolved {
            Resolved::Hit(target) => (
                target.provider.id,
                target.upstream_model,
                target.original_model,
            ),
            other => panic!("expected a hit, got {other:?}"),
        }
    }

    fn miss(resolved: Resolved) -> StackMiss {
        match resolved {
            Resolved::Miss(miss) => miss,
            other => panic!("expected a miss, got {other:?}"),
        }
    }

    #[test]
    fn a_claude_route_publishes_its_whole_list_and_leads_with_its_default() {
        let fx = fixture();
        let stack = state::stack(&fx.store, "claude").unwrap();
        let members =
            |route| published_members("ccs-", &fx.db, &AppType::Claude, &stack, route).unwrap();
        let ids = |route| {
            claude_published("ccs-", &members(route))
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>()
        };
        // 路由那家照常发布，和不在代理模式时算出来的一样。
        let all = ids(None);
        assert!(all.contains(&"ccs-claude-kimi--kimi-k3".to_string()));
        assert_eq!(ids(Some("kimi")), all);
        assert_eq!(claude_route_default("ccs-", &members(None)), None);
        let default = claude_route_default("ccs-", &members(Some("zhipu"))).unwrap();
        assert_eq!(
            (default.id.as_str(), default.name.as_str()),
            ("ccs-claude-zhipu--glm-5.2[1M]", "glm-5.2")
        );

        // 界面上标出路由那家，它的模型 id 照常列出。
        let views = member_views(
            &AppType::Claude,
            &super::members("ccs-", &fx.db, &AppType::Claude, &stack, Some("kimi")).unwrap(),
        );
        let route_flags: Vec<(&str, &str, bool, usize)> = views
            .iter()
            .map(|view| {
                (
                    view.provider_id.as_str(),
                    view.key.as_str(),
                    view.route,
                    view.model_ids.len(),
                )
            })
            .collect();
        assert_eq!(
            route_flags,
            vec![("kimi", "kimi", true, 1), ("zhipu", "zhipu", false, 1)]
        );
    }

    #[test]
    fn the_default_is_the_first_model_of_the_routes_list() {
        let fx = fixture();
        let kimi = with_stack_models(
            provider(
                "kimi",
                "Kimi",
                Some("kimi"),
                json!({ "ANTHROPIC_MODEL": "kimi-k3" }),
            ),
            json!([
                { "model": "kimi-k3-mini", "displayName": "K3 Mini" },
                { "model": "kimi-k3" }
            ]),
        );
        fx.db.save_provider("claude", &kimi).unwrap();
        let stack = state::stack(&fx.store, "claude").unwrap();
        let published =
            published_members("ccs-", &fx.db, &AppType::Claude, &stack, Some("kimi")).unwrap();
        let default = claude_route_default("ccs-", &published).unwrap();
        assert_eq!(
            (default.id.as_str(), default.name.as_str()),
            ("ccs-claude-kimi--kimi-k3-mini", "K3 Mini")
        );
        let ids: Vec<String> = claude_published("ccs-", &published)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "ccs-claude-kimi--kimi-k3-mini".to_string(),
                "ccs-claude-kimi--kimi-k3".to_string(),
                "ccs-claude-zhipu--glm-5.2[1M]".to_string(),
            ]
        );
    }

    #[test]
    fn route_roles_map_tiers_to_list_models_with_fallback_to_the_first() {
        let fx = fixture();
        let kimi = with_role_mapping(
            with_stack_models(
                provider(
                    "kimi",
                    "Kimi",
                    Some("kimi"),
                    json!({ "ANTHROPIC_MODEL": "kimi-k3" }),
                ),
                json!([
                    { "model": "kimi-k3" },
                    { "model": "kimi-k3-mini", "displayName": "K3 Mini", "oneM": true }
                ]),
            ),
            json!({
                // 带 1M 标记和首尾空格：去标记后命中列表行；id 的 1M 跟随行里的 oneM。
                "haiku": " kimi-k3-mini[1M] ",
                // 不在列表里的值静默回落第一模型。
                "opus": "not-in-list",
            }),
        );
        fx.db.save_provider("claude", &kimi).unwrap();
        let stack = state::stack(&fx.store, "claude").unwrap();
        let published =
            published_members("ccs-", &fx.db, &AppType::Claude, &stack, Some("kimi")).unwrap();
        let roles = claude_route_roles("ccs-", &published).unwrap();
        assert_eq!(roles.haiku.id, "ccs-claude-kimi--kimi-k3-mini[1M]");
        assert_eq!(roles.haiku.name, "K3 Mini");
        // 未配的 sonnet/fable 与配了但查不到的 opus 都回落第一模型。
        for tier in [&roles.sonnet, &roles.opus, &roles.fable] {
            assert_eq!(tier.id, "ccs-claude-kimi--kimi-k3");
            assert_eq!(tier.name, "kimi-k3");
        }
    }

    #[test]
    fn route_roles_use_the_env_derived_list_when_no_stack_models() {
        let fx = fixture();
        // 发布列表来自 env 档位推导（含 normalize 会回填的档位值），档位查表对象即该推导列表。
        let kimi = with_role_mapping(
            provider(
                "kimi",
                "Kimi",
                Some("kimi"),
                json!({
                    "ANTHROPIC_MODEL": "kimi-k3",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "kimi-k3-mini"
                }),
            ),
            json!({ "haiku": "kimi-k3-mini" }),
        );
        fx.db.save_provider("claude", &kimi).unwrap();
        let stack = state::stack(&fx.store, "claude").unwrap();
        let published =
            published_members("ccs-", &fx.db, &AppType::Claude, &stack, Some("kimi")).unwrap();
        let roles = claude_route_roles("ccs-", &published).unwrap();
        assert_eq!(roles.haiku.id, "ccs-claude-kimi--kimi-k3-mini");
        assert_eq!(roles.sonnet.id, "ccs-claude-kimi--kimi-k3");
    }

    #[test]
    fn route_roles_are_none_without_a_route_member() {
        let fx = fixture();
        let stack = state::stack(&fx.store, "claude").unwrap();
        let members =
            published_members("ccs-", &fx.db, &AppType::Claude, &stack, None).unwrap();
        assert!(claude_route_roles("ccs-", &members).is_none());
    }

    #[test]
    fn stacked_ids_resolve_to_their_provider_and_upstream_model() {
        let fx = fixture();
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude-kimi--kimi-k3")),
            (
                "kimi".to_string(),
                "kimi-k3".to_string(),
                "ccs-claude-kimi--kimi-k3".to_string()
            )
        );
        // 行里带 1M 标记的模型：不管客户端发来的 id 带不带标记，上游都用行里的原值。
        for id in ["ccs-claude-zhipu--glm-5.2", "ccs-claude-zhipu--glm-5.2[1m]"] {
            assert_eq!(
                hit(resolve_in(&fx, AppType::Claude, id)).1,
                "glm-5.2[1M]",
                "{id}"
            );
        }
        // 行里已经没有的模型照原样发。
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude-kimi--kimi-k9")).1,
            "kimi-k9"
        );
    }

    #[test]
    fn stacked_ids_that_cannot_be_resolved_never_fall_back_to_the_route() {
        let fx = fixture();
        let claude = |id| miss(resolve_in(&fx, AppType::Claude, id));
        assert_eq!(claude("ccs-claude-gone--g-1"), StackMiss::Removed);
        assert_eq!(claude("ccs-claude-deleted--d-1"), StackMiss::Deleted);
        assert_eq!(claude("ccs-claude-nobody--m"), StackMiss::Unknown);
        // fork: "ccs-kimi" 短形式现在解析为成员默认模型（见 short_form_resolves_member_default_model）
        // 名单为空（这个应用从没加过 Stack 模型）也一样报错。
        assert_eq!(
            miss(resolve_in(&fx, AppType::Codex, "ccs-claude-kimi--kimi-k3")),
            StackMiss::Unknown
        );
        let message = StackMiss::Removed.message("ccs-claude-gone--g-1");
        assert!(message.contains("ccs-claude-gone--g-1"), "{message}");
    }

    #[test]
    fn stacked_ids_are_rejected_outside_stack_mode() {
        let fx = fixture();
        let id = "ccs-claude-kimi--kimi-k3";
        assert!(matches!(
            resolve_in(&fx, AppType::Claude, id),
            Resolved::Hit(_)
        ));

        // 路由模式：名单和 key 都留着，客户端手里的旧 id 也不能转给名单里的那家。
        state::update(&fx.store, |live| {
            live.apps.get_mut("claude").unwrap().stack.enabled = false;
        })
        .unwrap();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, id)),
            StackMiss::StackOff
        );

        // 退回直连后附加位不动，同样不解析。
        state::update(&fx.store, |live| {
            let claude = live.apps.get_mut("claude").unwrap();
            claude.stack.enabled = true;
            claude.mode = Some(state::Mode::Direct);
        })
        .unwrap();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, id)),
            StackMiss::StackOff
        );
        let message = StackMiss::StackOff.message(id);
        assert!(message.contains(id), "{message}");
    }

    #[test]
    fn codex_ids_resolve_to_the_catalog_model_and_leave_the_routes_names_alone() {
        let fx = fixture();
        let mut deepseek = provider("ds", "DeepSeek", Some("deepseek"), json!({}));
        deepseek.settings_config = json!({
            "auth": {},
            "config": "model = \"deepseek-v4-flash\"\n",
            "modelCatalog": { "models": [{ "model": "deepseek-v4-pro" }] },
        });
        fx.db.save_provider("codex", &deepseek).unwrap();
        state::update(&fx.store, |live| {
            let stack = &mut live.apps.entry("codex".to_string()).or_default().stack;
            stack.members = vec!["ds".to_string()];
            stack.keys.insert("deepseek".to_string(), "ds".to_string());
        })
        .unwrap();

        assert_eq!(
            codex_model_ids("ccs-", "deepseek", &deepseek),
            vec!["ccs-deepseek/deepseek-v4-pro"]
        );
        assert_eq!(
            hit(resolve_in(
                &fx,
                AppType::Codex,
                "ccs-deepseek/deepseek-v4-pro"
            )),
            (
                "ds".to_string(),
                "deepseek-v4-pro".to_string(),
                "ccs-deepseek/deepseek-v4-pro".to_string()
            )
        );
        // 路由那家自己的 `deepseek/...`（OpenRouter 写法）不带保留前缀，照旧走默认路由。
        assert!(matches!(
            resolve_in(&fx, AppType::Codex, "deepseek/deepseek-v4-pro"),
            Resolved::Plain
        ));
    }

    #[test]
    fn plain_models_do_not_read_any_state() {
        let fx = fixture();
        // 状态文件坏了也不影响普通请求：不带保留前缀时根本不读。
        std::fs::write(fx.store.state_path(), "{ not json").unwrap();
        for (app, model) in [
            (AppType::Claude, "claude-sonnet-5"),
            (AppType::Claude, "kimi-k3"),
            (AppType::Codex, "deepseek/deepseek-v4-pro"),
            (AppType::ClaudeDesktop, "ccs-claude-kimi--kimi.kimi-k3"),
        ] {
            assert!(
                matches!(resolve_in(&fx, app, model), Resolved::Plain),
                "{model}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(fx.store.state_path()).unwrap(),
            "{ not json"
        );
    }

    // fork: 前缀可配置 + 短形式 id（doc/20261009-设计文档-会话路由改造为聚合模式）

    #[test]
    fn claude_full_id_with_custom_prefix() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.claude-cc--m"),
            Decoded::Stack {
                key: "cc",
                model: "m",
                one_m: false
            }
        );
    }

    #[test]
    fn codex_full_id_with_custom_prefix() {
        assert_eq!(
            decode("G.", &AppType::Codex, "G.cc/m"),
            Decoded::Stack {
                key: "cc",
                model: "m",
                one_m: false
            }
        );
    }

    #[test]
    fn claude_short_id() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.cc"),
            Decoded::Short { key: "cc" }
        );
    }

    #[test]
    fn codex_prefix_without_separator_is_short() {
        // 旧行为是 Plain（普通模型名透传）；短形式语义下带前缀即聚合寻址
        assert_eq!(
            decode("ccs-", &AppType::Codex, "ccs-foo"),
            Decoded::Short { key: "foo" }
        );
    }

    #[test]
    fn claude_short_reserved_default() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.default"),
            Decoded::Short { key: "default" }
        );
        assert_eq!(
            decode("G.", &AppType::Codex, "G.default"),
            Decoded::Short { key: "default" }
        );
    }

    #[test]
    fn empty_key_is_malformed() {
        assert_eq!(decode("G.", &AppType::Claude, "G."), Decoded::Malformed);
        assert_eq!(decode("G.", &AppType::Codex, "G."), Decoded::Malformed);
    }

    #[test]
    fn one_m_marker_stripped_on_custom_prefix() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.claude-cc--m[1M]"),
            Decoded::Stack {
                key: "cc",
                model: "m",
                one_m: true
            }
        );
    }

    #[test]
    fn plain_without_prefix_under_custom_prefix() {
        assert_eq!(decode("G.", &AppType::Claude, "glm-5.3"), Decoded::Plain);
        assert_eq!(decode("G.", &AppType::Codex, "gpt-5.2"), Decoded::Plain);
        // 配置为 G. 时，ccs- 前缀的 id 只是普通模型名（前缀不匹配 → Plain）
        assert_eq!(decode("G.", &AppType::Codex, "ccs-claude-k--m"), Decoded::Plain);
    }

    #[test]
    fn encode_roundtrip_custom_prefix() {
        assert_eq!(encode("G.", &AppType::Claude, "cc", "m", true), "G.claude-cc--m[1M]");
        assert_eq!(encode("G.", &AppType::Codex, "cc", "m", false), "G.cc/m");
        assert_eq!(encode_short("G.", &AppType::Claude, "cc"), "G.cc");
        assert_eq!(encode_short("G.", &AppType::Codex, "cc"), "G.cc");
        assert_eq!(encode_short("ccs-", &AppType::Claude, "cc"), "ccs-cc");
    }

    #[test]
    fn provider_label_prefers_trimmed_short_name() {
        let base = provider("p", "智谱 GLM", None, json!({}));
        assert_eq!(provider_label(&base), "智谱 GLM");
        let mut blank = base.clone();
        blank.meta = Some(crate::provider::ProviderMeta {
            short_name: Some("   ".to_string()),
            ..Default::default()
        });
        assert_eq!(provider_label(&blank), "智谱 GLM");
        let mut short = base;
        short.meta = Some(crate::provider::ProviderMeta {
            short_name: Some("  智谱  ".to_string()),
            ..Default::default()
        });
        assert_eq!(provider_label(&short), "智谱");
    }

    #[test]
    fn display_name_uses_ascii_parens_without_space() {
        assert_eq!(display_name("GLM-5.3", "智谱"), "GLM-5.3(智谱)");
    }

    // fork Task 2: 短形式解析 → 成员默认模型

    #[test]
    fn short_form_resolves_member_default_model() {
        let fx = fixture();
        // 映射行：默认模型 = ANTHROPIC_MODEL（upstream 原值，1M 标记保留）
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-kimi")),
            (
                "kimi".to_string(),
                "kimi-k3".to_string(),
                "ccs-kimi".to_string()
            )
        );
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-zhipu")).1,
            "glm-5.2[1M]"
        );
    }

    #[test]
    fn short_form_uses_first_listed_stack_model() {
        let fx = fixture();
        let kimi = with_stack_models(
            provider(
                "kimi",
                "Kimi",
                Some("kimi"),
                json!({ "ANTHROPIC_MODEL": "kimi-k3" }),
            ),
            json!([
                { "model": "kimi-k3-mini", "displayName": "K3 Mini" },
                { "model": "kimi-k3" }
            ]),
        );
        fx.db.save_provider("claude", &kimi).unwrap();
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-kimi")).1,
            "kimi-k3-mini"
        );
    }

    #[test]
    fn short_form_codex_reads_model_field() {
        let fx = fixture();
        let mut deepseek = provider("ds", "DeepSeek", Some("deepseek"), json!({}));
        deepseek.settings_config = json!({
            "auth": {},
            "config": "model = \"deepseek-v4-flash\"\n",
            "modelCatalog": { "models": [{ "model": "deepseek-v4-pro" }] },
        });
        fx.db.save_provider("codex", &deepseek).unwrap();
        state::update(&fx.store, |live| {
            let stack = &mut live.apps.entry("codex".to_string()).or_default().stack;
            stack.members = vec!["ds".to_string()];
            stack.keys.insert("deepseek".to_string(), "ds".to_string());
        })
        .unwrap();
        assert_eq!(
            hit(resolve_in(&fx, AppType::Codex, "ccs-deepseek")),
            (
                "ds".to_string(),
                "deepseek-v4-flash".to_string(),
                "ccs-deepseek".to_string()
            )
        );
    }

    #[test]
    fn short_form_member_without_default_model_misses() {
        let fx = fixture();
        let bare = provider("bare", "Bare", None, json!({}));
        fx.db.save_provider("claude", &bare).unwrap();
        state::update(&fx.store, |live| {
            let stack = &mut live.apps.entry("claude".to_string()).or_default().stack;
            stack.members.push("bare".to_string());
            stack.keys.insert("bare".to_string(), "bare".to_string());
        })
        .unwrap();
        let miss = miss(resolve_in(&fx, AppType::Claude, "ccs-bare"));
        assert_eq!(miss, StackMiss::NoDefaultModel);
        let message = StackMiss::NoDefaultModel.message("ccs-bare");
        assert!(message.contains("默认模型"), "{message}");
    }

    #[test]
    fn short_default_key_untouched_by_resolve() {
        // 保留 key default 的解绑语义在代理层（route_prefix::apply_session_routing），
        // resolve 不实现——钉住该边界
        let fx = fixture();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-default")),
            StackMiss::Unknown
        );
    }

    #[test]
    fn short_form_removed_and_deleted_members_miss() {
        let fx = fixture();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-gone")),
            StackMiss::Removed
        );
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-deleted")),
            StackMiss::Deleted
        );
    }

    #[test]
    fn short_form_outside_stack_mode_misses() {
        let fx = fixture();
        state::update(&fx.store, |live| {
            live.apps.get_mut("claude").unwrap().stack.enabled = false;
        })
        .unwrap();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-kimi")),
            StackMiss::StackOff
        );
    }

    // fork Task 5: 自定义 key（改名 + 墓碑）

    #[test]
    fn validate_member_key_matrix() {
        let mut stack = StackState::default();
        stack.keys.insert("kimi".to_string(), "kimi".to_string());
        stack.reserved_keys.insert("old".to_string());
        // 视角：改名者是 deepseek（登记簿/墓簿里没有它的条目，所有冲突都是别人的）
        let self_id = "deepseek";

        // 归一化：大小写保留、非法字符换横线、连续横线合并（doc/20261009-五项优化）
        assert_eq!(
            validate_member_key(&stack, "CommandCode", self_id),
            Ok("CommandCode".to_string())
        );
        assert_eq!(
            validate_member_key(&stack, "a--b", self_id),
            Ok("a-b".to_string())
        );
        assert_eq!(
            validate_member_key(&stack, "A_B", self_id),
            Ok("A-B".to_string())
        );
        // `.` 是 id 的 key↔模型分隔符，归一化为横线
        assert_eq!(
            validate_member_key(&stack, "my.key", self_id),
            Ok("my-key".to_string())
        );
        // 超长由 slug 截断到 24 位，不报错
        assert_eq!(
            validate_member_key(&stack, &"x".repeat(40), self_id),
            Ok("x".repeat(24))
        );

        // 归一化后为空 / 保留字（忽略大小写——decode 端解绑判断本就忽略）/
        // 与登记簿冲突（忽略大小写，避免视觉重复分组）；墓碑不挡手动改名
        assert!(validate_member_key(&stack, "", self_id).is_err());
        assert!(validate_member_key(&stack, "///", self_id).is_err());
        assert!(validate_member_key(&stack, "default", self_id).is_err());
        assert!(validate_member_key(&stack, "Default", self_id).is_err());
        assert!(validate_member_key(&stack, "Kimi", self_id).is_err());
        assert!(validate_member_key(&stack, "KIMI", self_id).is_err());
        assert_eq!(
            validate_member_key(&stack, "old", self_id),
            Ok("old".to_string())
        );
    }

    #[test]
    fn renaming_to_a_case_variant_of_ones_own_key_is_allowed() {
        // 用户把 key 从旧 slug 时代的小写升级成大小写（zhipupro → ZhiPuPro）：
        // 登记簿里指向自己的条目豁免，仅大小写变化不算冲突；别人用同形仍被挡；
        // 墓碑不挡手动改名（fork 五项优化验收后收敛：墓碑只防自动分配，手动
        // 复用旧 key 由用户自负责，后果写进 UI hint）
        let mut stack = StackState::default();
        stack
            .keys
            .insert("zhipupro".to_string(), "zhipu".to_string());
        stack.keys.insert("kimi".to_string(), "kimi".to_string());
        stack.reserved_keys.insert("oldkey".to_string());
        stack.reserved_keys.insert("zhipumax".to_string());

        // 自己改大小写变体：通过
        assert_eq!(
            validate_member_key(&stack, "ZhiPuPro", "zhipu"),
            Ok("ZhiPuPro".to_string())
        );
        // 别人用同样的大小写变体：挡（撞 zhipu 的现 key，防模型列表视觉重复）
        assert!(validate_member_key(&stack, "ZhiPuPro", "kimi").is_err());
        // 墓碑里的旧 key 手动复用：全部允许（精确改回、大小写变体）
        assert_eq!(
            validate_member_key(&stack, "oldkey", "zhipu"),
            Ok("oldkey".to_string())
        );
        assert_eq!(
            validate_member_key(&stack, "zhipumax", "zhipu"),
            Ok("zhipumax".to_string())
        );
        assert_eq!(
            validate_member_key(&stack, "ZhiPuMax", "zhipu"),
            Ok("ZhiPuMax".to_string())
        );
    }

    #[test]
    fn auto_allocated_keys_stay_lowercase_even_amid_uppercase_ones() {
        // 手动改出的大写 key 占位后，自动分配同名小写 key 要顺延（去重忽略大小写），
        // 且自动分配本身永远产小写（与历史 key 风格一致）
        let mut stack = StackState::default();
        stack.keys.insert("Zhipu".to_string(), "zhipu".to_string());
        let provider =
            Provider::with_id("zhipu2".to_string(), "Zhipu".to_string(), json!({}), None);
        assert_eq!(allocate_key(&mut stack, &provider), "zhipu-2");
    }

    #[test]
    fn renamed_key_tombstones_the_old_one() {
        // 模拟 controller 改名后的登记簿：新 key 指向成员，旧 key 进墓碑
        let mut stack = StackState::default();
        stack.members = vec!["kimi".to_string()];
        stack.keys.insert("code".to_string(), "kimi".to_string());
        stack.reserved_keys.insert("kimi".to_string());

        let db = Database::memory().unwrap();
        db.save_provider(
            "claude",
            &provider(
                "kimi",
                "Kimi",
                Some("kimi"),
                json!({ "ANTHROPIC_MODEL": "m" }),
            ),
        )
        .unwrap();
        // 旧 key 解析不到（Unknown），新 key 正常
        assert!(matches!(
            resolve_member(&db, &stack, &AppType::Claude, "kimi"),
            Ok(Err(StackMiss::Unknown))
        ));
        assert!(matches!(
            resolve_member(&db, &stack, &AppType::Claude, "code"),
            Ok(Ok(_))
        ));

        // 墓碑 key 不被 allocate_key 再分配：同图标的新家拿 kimi-2
        let other = provider("b", "Kimi 2", Some("kimi"), json!({}));
        assert_eq!(allocate_key(&mut stack, &other), "kimi-2");
    }

    mod claude_desktop {
        use super::*;
        use crate::provider::{ClaudeDesktopMode, ClaudeDesktopModelRoute, ProviderMeta};

        /// Desktop 2.31226.1 选择器的过滤（`bo`）：命中厂商黑名单就丢，否则 id 要是纯档位别名或
        /// 含 claude 等字样。黑名单随 Desktop 版本变长，升级后从 app.asar 里重新抄一遍。
        fn desktop_keeps(id: &str) -> bool {
            let blacklist = regex::Regex::new(
                r"ark-code|astron|command-r|deepseek|doubao|gemini|gemma|glm|gpt|grok|hermes|hy3|kimi|lfm|\bling\b|llama|longcat|mimo|minimax|mistral|mixtral|moonshot|nemotron|openai|phi-|qianfan|qwen|tc-code|\bunic\b|yi-|stepfun|step-3|seed-|bytedance|hunyuan|granite|amazon\.nova|nova-|devstral|ministral|ernie|codex|arcee|trinity|abab|phi\d|\bk2\.|\bm2\.|jamba|arctic|solar|mercury|zamba|kat-coder|\bds-|dpsk",
            )
            .unwrap();
            let id = id.to_lowercase();
            !blacklist.is_match(&id)
                && [
                    "claude",
                    "sonnet",
                    "opus",
                    "haiku",
                    "fable",
                    "mythos",
                    "anthropic",
                ]
                .iter()
                .any(|word| id.contains(word))
        }

        /// Desktop 归一化模型身份时剥掉末尾的 `-<8 位数字>`（日期）。
        fn desktop_identity(id: &str) -> String {
            let id = id.to_lowercase();
            regex::Regex::new(r"[-@]\d{8}$")
                .unwrap()
                .replace(&id, "")
                .into_owned()
        }

        fn card(id: &str, name: &str, routes: &[(&str, &str, Option<&str>, bool)]) -> Provider {
            let mut provider = Provider::with_id(
                id.to_string(),
                name.to_string(),
                json!({ "env": {
                    "ANTHROPIC_BASE_URL": format!("https://{id}.example"),
                    "ANTHROPIC_AUTH_TOKEN": format!("sk-{id}"),
                }}),
                None,
            );
            provider.meta = Some(ProviderMeta {
                claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
                claude_desktop_model_routes: routes
                    .iter()
                    .map(|(route, model, label, one_m)| {
                        (
                            route.to_string(),
                            ClaudeDesktopModelRoute {
                                model: model.to_string(),
                                label_override: label.map(str::to_string),
                                supports_1m: one_m.then_some(true),
                            },
                        )
                    })
                    .collect(),
                ..Default::default()
            });
            provider
        }

        /// Desktop 在聚合模式。kimi、zhipu 在名单里；gone 登记过但已移除。
        fn fixture() -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let store = DeviceStore::at(dir.path());
            let db = Database::memory().unwrap();
            for row in [
                card(
                    "kimi",
                    "Kimi",
                    &[
                        ("claude-sonnet-5", "kimi-k3", Some("Kimi K3"), true),
                        ("claude-haiku-4-5", "kimi-k3-turbo", None, false),
                    ],
                ),
                card(
                    "zhipu",
                    "Zhipu",
                    &[("claude-sonnet-5", "glm-5.2", None, false)],
                ),
                card("gone", "Gone", &[("claude-sonnet-5", "g-1", None, false)]),
            ] {
                db.save_provider("claude-desktop", &row).unwrap();
            }
            state::update(&store, |live| {
                let desktop = live.apps.entry("claude-desktop".to_string()).or_default();
                desktop.mode = Some(state::Mode::Proxy);
                desktop.proxy_route = Some("zhipu".to_string());
                let stack = &mut desktop.stack;
                stack.enabled = true;
                stack.members = vec!["zhipu".to_string(), "kimi".to_string()];
                for key in ["kimi", "zhipu", "gone"] {
                    stack.keys.insert(key.to_string(), key.to_string());
                }
            })
            .unwrap();
            Fixture {
                _dir: dir,
                store,
                db,
            }
        }

        #[test]
        fn ids_are_digits_only_and_survive_the_desktop_picker() {
            let id = encode("", &AppType::ClaudeDesktop, "kimi", "kimi-k3", true);
            assert_eq!(
                id,
                encode("", &AppType::ClaudeDesktop, "kimi", "kimi-k3", false)
            );
            assert_ne!(
                id,
                encode("", &AppType::ClaudeDesktop, "kimi", "kimi-k3-turbo", false)
            );
            assert_ne!(
                id,
                encode("", &AppType::ClaudeDesktop, "kimi-2", "kimi-k3", false)
            );
            // 散列写死，换算法会让客户端手里的 id 失效
            assert_eq!(id, "claude-fable-ccs-473908");

            let keys = [
                "kimi",
                "zhipu",
                "deepseek",
                "seed",
                "my-relay-2-0",
                "p1a2b3",
            ];
            let models = [
                "kimi-k3",
                "glm-5.2",
                "deepseek-v4-pro",
                "seed-oss",
                "gpt-5.5",
                "k2.5",
            ];
            for key in keys {
                for model in models {
                    let id = encode("", &AppType::ClaudeDesktop, key, model, false);
                    let digits = id.strip_prefix(DESKTOP_PREFIX).unwrap();
                    assert_eq!(digits.len(), DESKTOP_CODE_DIGITS, "{id}");
                    assert!(digits.bytes().all(|b| b.is_ascii_digit()), "{id}");
                    assert!(desktop_keeps(&id), "Desktop would drop {id}");
                    assert_eq!(desktop_identity(&id), id, "Desktop would trim {id}");
                    assert_eq!(decode("", &AppType::ClaudeDesktop, &id), Decoded::Code(digits));
                }
            }
            // Claude Code 的写法 Desktop 会丢掉，所以不能照搬
            assert!(!desktop_keeps("ccs-claude-kimi--kimi-k3"));
        }

        #[test]
        fn decoding_takes_the_1m_row_and_rejects_anything_but_six_digits() {
            let app = AppType::ClaudeDesktop;
            assert_eq!(
                decode("", &app, "claude-fable-ccs-012345[1m]"),
                Decoded::Code("012345")
            );
            assert_eq!(decode("", &app, "claude-fable-ccs-1234"), Decoded::Malformed);
            assert_eq!(
                decode("", &app, "claude-fable-ccs-12345678"),
                Decoded::Malformed
            );
            assert_eq!(decode("", &app, "claude-fable-ccs-12345a"), Decoded::Malformed);
            assert_eq!(decode("", &app, "claude-sonnet-5"), Decoded::Plain);
            assert_eq!(decode("", &app, "claude-fable-5"), Decoded::Plain);
            // 别的应用不认这个前缀
            assert_eq!(
                decode("ccs-", &AppType::Claude, "claude-fable-ccs-123456"),
                Decoded::Plain
            );
        }

        #[test]
        fn members_publish_their_mapped_models_and_the_route_publishes_none() {
            let fx = fixture();
            let stack = state::stack(&fx.store, "claude-desktop").unwrap();
            let members =
                published_members("", &fx.db, &AppType::ClaudeDesktop, &stack, Some("zhipu")).unwrap();
            let published = desktop_published(&members);
            assert_eq!(
                published
                    .iter()
                    .map(|model| (
                        model.upstream.as_str(),
                        model.label.as_str(),
                        model.supports_1m
                    ))
                    .collect::<Vec<_>>(),
                vec![
                    ("kimi-k3-turbo", "Kimi / kimi-k3-turbo", false),
                    ("kimi-k3", "Kimi / Kimi K3", true),
                ]
            );
            assert!(published.iter().all(|model| desktop_keeps(&model.id)));
            // 默认那家现在不发布，但换默认的预览要知道它换走后会发布几个
            let views = member_views(
                &AppType::ClaudeDesktop,
                &super::super::members("", &fx.db, &AppType::ClaudeDesktop, &stack, Some("zhipu"))
                    .unwrap(),
            );
            let counts: Vec<(&str, usize, usize)> = views
                .iter()
                .map(|view| {
                    (
                        view.provider_id.as_str(),
                        view.model_ids.len(),
                        view.member_models,
                    )
                })
                .collect();
            assert_eq!(counts, vec![("zhipu", 0, 1), ("kimi", 2, 2)]);

            // 默认换成 kimi 后，zhipu 的模型带数字 id 发布，kimi 的不再发布
            let members =
                published_members("", &fx.db, &AppType::ClaudeDesktop, &stack, Some("kimi")).unwrap();
            assert_eq!(
                desktop_published(&members)
                    .into_iter()
                    .map(|model| model.upstream)
                    .collect::<Vec<_>>(),
                vec!["glm-5.2"]
            );
        }

        #[test]
        fn ids_resolve_to_the_member_and_never_fall_back_to_the_route() {
            let fx = fixture();
            let app = AppType::ClaudeDesktop;
            let kimi = encode("", &app, "kimi", "kimi-k3", false);
            assert_eq!(
                hit(resolve_in(&fx, app.clone(), &format!("{kimi}[1m]"))),
                (
                    "kimi".to_string(),
                    "kimi-k3".to_string(),
                    format!("{kimi}[1m]")
                )
            );
            // 路由那家自己的数字 id 也认（它做默认前客户端可能选过）
            let zhipu = encode("", &app, "zhipu", "glm-5.2", false);
            assert_eq!(hit(resolve_in(&fx, app.clone(), &zhipu)).1, "glm-5.2");

            let gone = encode("", &app, "gone", "g-1", false);
            assert_eq!(
                miss(resolve_in(&fx, app.clone(), &gone)),
                StackMiss::Removed
            );
            assert_eq!(
                miss(resolve_in(
                    &fx,
                    app.clone(),
                    &encode("", &app, "kimi", "kimi-k9", false)
                )),
                StackMiss::Unknown
            );
            assert_eq!(
                miss(resolve_in(&fx, app.clone(), "claude-fable-ccs-12")),
                StackMiss::Unknown
            );
            // 路由那家的映射表 id 不经过这里
            assert!(matches!(
                resolve_in(&fx, app.clone(), "claude-sonnet-5"),
                Resolved::Plain
            ));

            state::update(&fx.store, |live| {
                live.apps.get_mut("claude-desktop").unwrap().stack.enabled = false;
            })
            .unwrap();
            assert_eq!(miss(resolve_in(&fx, app, &kimi)), StackMiss::StackOff);
        }

        #[test]
        fn a_colliding_id_resolves_to_the_published_member_not_an_earlier_route() {
            let app = AppType::ClaudeDesktop;
            let id = encode("", &app, "relay-1944", "kimi-k3", false);
            assert_eq!(id, encode("", &app, "relay-484", "kimi-k3", false));

            let dir = tempfile::tempdir().unwrap();
            let fx = Fixture {
                store: DeviceStore::at(dir.path()),
                db: Database::memory().unwrap(),
                _dir: dir,
            };
            for key in ["relay-484", "relay-1944"] {
                let row = card(key, key, &[("claude-sonnet-5", "kimi-k3", None, false)]);
                fx.db.save_provider("claude-desktop", &row).unwrap();
            }
            state::update(&fx.store, |live| {
                let desktop = live.apps.entry("claude-desktop".to_string()).or_default();
                desktop.mode = Some(state::Mode::Proxy);
                desktop.proxy_route = Some("relay-484".to_string());
                let stack = &mut desktop.stack;
                stack.enabled = true;
                // 路由那家排在前面
                stack.members = vec!["relay-484".to_string(), "relay-1944".to_string()];
                for key in ["relay-484", "relay-1944"] {
                    stack.keys.insert(key.to_string(), key.to_string());
                }
            })
            .unwrap();

            let stack = state::stack(&fx.store, "claude-desktop").unwrap();
            let members = published_members("", &fx.db, &app, &stack, Some("relay-484")).unwrap();
            let published = desktop_published(&members);
            assert_eq!(published.len(), 1);
            assert_eq!(published[0].label, "relay-1944 / kimi-k3");
            assert_eq!(hit(resolve_in(&fx, app, &id)).0, "relay-1944");
        }
    }
}
