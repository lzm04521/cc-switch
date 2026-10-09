//! Stack 模式（界面和代码都叫 Stack）。
//!
//! Stack 模式和路由模式在界面上二选一，内部都是代理模式：Stack 模式多一个开关位
//! （[`StackState::enabled`]）。Stack 模式下供应商列表是累加式的：添加的每一家（第三方）的
//! 模型以带保留前缀的 id 发布给客户端，选中后请求直达那一家；不带前缀的请求发往「默认」
//! 那家（代理路由），不做故障转移。Claude Code 的四档别名（启动默认、后台任务、子代理别名）
//! 都指向默认那家列表里的第一个模型（[`claude_route_default`]），平时用哪个由用户在
//! `/model` 里选。
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
// 上游默认前缀是 `ccs-`（Claude 为 `ccs-claude.`）；用户可经设置改成 G. 等。生产调用方统一经
// [`configured_prefix`] 取值一次显式传入——不在这层读全局 settings：settings_store 是 OnceLock
// 惰性读真实磁盘，掺进纯函数会让测试结果机器相关（开发机配置了 G. 就全挂）。
/// 配置的模型 id 前缀（settings 缺省/非法时回退上游默认，见 `route_prefix::DEFAULT_ROUTE_PREFIX`）。
pub fn configured_prefix() -> String {
    crate::settings::get_route_prefix()
}

/// Claude 的 Stack 模型 id：`<prefix>claude.<key>.<model>`。id 里要有 `claude` 才进
/// `/model` 选择器，不以 `claude-` 开头 MAX 窗口才生效（`claude` 中缀因此保留）。
const CLAUDE_SEPARATOR: &str = ".";
/// Codex 的 Stack 模型 id：`<prefix><key>.<model>`。
const CODEX_SEPARATOR: &str = ".";

/// key 的最大长度：只是为了模型 id 不至于太长。
const KEY_MAX_LEN: usize = 24;

/// Claude Code 在没有 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 时按这个窗口算。
pub const CLAUDE_DEFAULT_WINDOW: u64 = 200_000;

/// 支持 Stack 模型的应用。
pub fn supports_stack(app: &AppType) -> bool {
    matches!(app, AppType::Claude | AppType::Codex)
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
    let base = [provider.icon.as_deref(), Some(provider.name.as_str())]
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
/// 忽略，精确匹配会让 `Default` 绕过校验却被代理当解绑 id）、不与登记簿/墓簿冲突
/// （同样忽略大小写：`Zhipu` 与 `zhipu` 并存会在模型列表里视觉重复）。
/// 返回归一化后的 key。
pub fn validate_member_key(stack: &StackState, raw: &str) -> Result<String, String> {
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
    if key_taken(stack, &normalized) {
        return Err(format!(
            "分组 key「{normalized}」已被使用（含改名留下的旧 key）(Key \"{normalized}\" is already taken)"
        ));
    }
    Ok(normalized)
}

/// key 是否已被占用（大小写不敏感）。登记簿、墓簿统一走这里。
fn key_taken(stack: &StackState, key: &str) -> bool {
    stack
        .keys
        .keys()
        .chain(stack.reserved_keys.iter())
        .any(|existing| existing.eq_ignore_ascii_case(key))
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

/// Stack 模型 id。Claude 的上游是 1M 窗口时末尾带 `[1M]`，Claude Code 才按 1M 计算。
pub fn encode(prefix: &str, app: &AppType, key: &str, model: &str, one_m: bool) -> String {
    match app {
        AppType::Codex => format!("{prefix}{key}{CODEX_SEPARATOR}{model}"),
        _ => {
            let marker = if one_m { ONE_M_MARKER_FOR_CLIENT } else { "" };
            format!("{prefix}claude.{key}{CLAUDE_SEPARATOR}{model}{marker}")
        }
    }
}

/// fork: 成员短形式 id（仅 key，选中即走该成员默认模型；保留 key `default` 为解绑语义）。
pub fn encode_short(prefix: &str, app: &AppType, key: &str) -> String {
    match app {
        AppType::Codex => format!("{prefix}{key}"),
        _ => format!("{prefix}claude.{key}"),
    }
}

/// 解码的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded<'a> {
    /// 不带保留前缀：普通模型名，照旧走代理路由。
    Plain,
    /// 带保留前缀，但切不出 key 和模型。
    Malformed,
    /// 带保留前缀。`model` 可以含 `.`（key 里没有这个分隔符，在第一个处切开）。
    Stack {
        key: &'a str,
        model: &'a str,
        /// Claude id 末尾带着 1M 标记。
        one_m: bool,
    },
    /// fork: 带保留前缀但没有分隔符——成员「默认模型」短形式 id（保留 key `default`
    /// 由代理层解绑语义处理，见 `route_prefix::apply_session_routing`）。
    Short { key: &'a str },
}

/// 按客户端的格式解码模型 id。Claude：`<prefix>claude.` 开头；Codex：`<prefix>` 开头。
/// 带前缀但没有分隔符的是短形式（fork：Codex 旧行为是当普通模型名 Plain，已反转）。
pub fn decode<'a>(prefix: &str, app: &AppType, id: &'a str) -> Decoded<'a> {
    let (rest, separator, one_m) = match app {
        AppType::Claude => {
            let claude_prefix = format!("{prefix}claude.");
            let Some(rest) = id.strip_prefix(&claude_prefix) else {
                return Decoded::Plain;
            };
            let stripped = strip_one_m_suffix_for_upstream(rest);
            (stripped, CLAUDE_SEPARATOR, stripped.len() != rest.len())
        }
        AppType::Codex => {
            let Some(rest) = id.strip_prefix(prefix) else {
                return Decoded::Plain;
            };
            (rest, CODEX_SEPARATOR, false)
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
}

/// 按去掉 1M 标记后的名字去重地加入 `found`。同一个模型有一处带 1M 标记就按 1M，发往上游的
/// 也用带标记的那个写法；显示名取第一个有的。
fn push_found(found: &mut Vec<Found>, upstream: &str, name: Option<String>) {
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
        }
        None => found.push(Found {
            model,
            upstream: upstream.to_string(),
            name,
            one_m,
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
        push_found(&mut found, &upstream, name);
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
        push_found(&mut found, upstream, name);
    }
    found
}

/// 给找到的模型加上前缀、显示名和窗口。非 1M 模型的窗口是行里的
/// `CLAUDE_CODE_MAX_CONTEXT_TOKENS`（Claude Code 只有一个全局窗口，没法按模型设），没有是 200K。
fn stack_models(
    prefix: &str,
    key: &str,
    provider: &Provider,
    found: Vec<Found>,
) -> Vec<StackModel> {
    let window = claude_env(provider)
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
            let shown_window = if found.one_m { 1_000_000 } else { window };
            StackModel {
                id: encode(prefix, &AppType::Claude, key, &found.model, found.one_m),
                display_name: display_name(&name, &provider.name),
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

/// 选择器里 Stack 模型的显示名：`<模型显示名>（<供应商名>）`。
pub fn display_name(model: &str, provider_name: &str) -> String {
    format!("{model}（{provider_name}）")
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
    /// 暂不可用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<&'static str>,
    /// Codex 客户端还在用旧的模型列表（启动时读的目录），Stack 模型看不到，要重启才行。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_clients: Option<StaleClients>,
}

pub fn member_views(members: &[Member]) -> Vec<StackMemberView> {
    members
        .iter()
        .map(|member| StackMemberView {
            provider_id: member.provider.id.clone(),
            key: member.key.clone(),
            model_ids: member.model_ids.clone(),
            route: member.route,
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

        assert_eq!(allocate_key(&mut stack, &kimi), "kimi");
        assert_eq!(allocate_key(&mut stack, &named), "my-relay-2-0");
        assert_eq!(allocate_key(&mut stack, &chinese), "pc1b2c3");
        assert_eq!(allocate_key(&mut stack, &blank_icon), "deepseek");
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
            decode("ccs-", &claude, "ccs-claude.k.m[1m]"),
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
        assert_eq!(id, "ccs-deepseek.deepseek/deepseek-v4-pro");
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
            decode("ccs-", &AppType::ClaudeDesktop, "ccs-claude.k.m"),
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
        assert_eq!(decode("ccs-", &claude, "ccs-claude..m"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &claude, "ccs-claude.k."), Decoded::Malformed);
        assert_eq!(decode("ccs-", &codex, "ccs-.m"), Decoded::Malformed);
        assert_eq!(decode("ccs-", &codex, "ccs-k."), Decoded::Malformed);
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
                    id: "ccs-claude.zhipu.glm-5.2[1M]".to_string(),
                    upstream: "glm-5.2[1M]".to_string(),
                    name: "GLM 5.2".to_string(),
                    display_name: "GLM 5.2（Zhipu）".to_string(),
                    description: "glm-5.2 · 1M".to_string(),
                    one_m: true,
                    window: 128_000,
                },
                StackModel {
                    id: "ccs-claude.zhipu.glm-4.7-air".to_string(),
                    upstream: "glm-4.7-air".to_string(),
                    name: "glm-4.7-air".to_string(),
                    display_name: "glm-4.7-air（Zhipu）".to_string(),
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
        assert_eq!(ids, vec!["ccs-claude.kimi.kimi-k3"]);
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
                    "ccs-claude.kimi.kimi-k3[1M]".to_string(),
                    "kimi-k3[1M]".to_string(),
                    "Kimi K3（Kimi）".to_string(),
                    true,
                    256_000,
                ),
                (
                    "ccs-claude.kimi.kimi-k3-thinking[1M]".to_string(),
                    "kimi-k3-thinking[1M]".to_string(),
                    "kimi-k3-thinking（Kimi）".to_string(),
                    true,
                    256_000,
                ),
            ]
        );
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
        assert!(all.contains(&"ccs-claude.kimi.kimi-k3".to_string()));
        assert_eq!(ids(Some("kimi")), all);
        assert_eq!(claude_route_default("ccs-", &members(None)), None);
        let default = claude_route_default("ccs-", &members(Some("zhipu"))).unwrap();
        assert_eq!(
            (default.id.as_str(), default.name.as_str()),
            ("ccs-claude.zhipu.glm-5.2[1M]", "glm-5.2")
        );

        // 界面上标出路由那家，它的模型 id 照常列出。
        let views = member_views(
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
            ("ccs-claude.kimi.kimi-k3-mini", "K3 Mini")
        );
        let ids: Vec<String> = claude_published("ccs-", &published)
            .into_iter()
            .map(|model| model.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "ccs-claude.kimi.kimi-k3-mini".to_string(),
                "ccs-claude.kimi.kimi-k3".to_string(),
                "ccs-claude.zhipu.glm-5.2[1M]".to_string(),
            ]
        );
    }

    #[test]
    fn stacked_ids_resolve_to_their_provider_and_upstream_model() {
        let fx = fixture();
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude.kimi.kimi-k3")),
            (
                "kimi".to_string(),
                "kimi-k3".to_string(),
                "ccs-claude.kimi.kimi-k3".to_string()
            )
        );
        // 行里带 1M 标记的模型：不管客户端发来的 id 带不带标记，上游都用行里的原值。
        for id in ["ccs-claude.zhipu.glm-5.2", "ccs-claude.zhipu.glm-5.2[1m]"] {
            assert_eq!(
                hit(resolve_in(&fx, AppType::Claude, id)).1,
                "glm-5.2[1M]",
                "{id}"
            );
        }
        // 行里已经没有的模型照原样发。
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude.kimi.kimi-k9")).1,
            "kimi-k9"
        );
    }

    #[test]
    fn stacked_ids_that_cannot_be_resolved_never_fall_back_to_the_route() {
        let fx = fixture();
        let claude = |id| miss(resolve_in(&fx, AppType::Claude, id));
        assert_eq!(claude("ccs-claude.gone.g-1"), StackMiss::Removed);
        assert_eq!(claude("ccs-claude.deleted.d-1"), StackMiss::Deleted);
        assert_eq!(claude("ccs-claude.nobody.m"), StackMiss::Unknown);
        // fork: "ccs-claude.kimi" 短形式现在解析为成员默认模型（见 short_form_resolves_member_default_model）
        // 名单为空（这个应用从没加过 Stack 模型）也一样报错。
        assert_eq!(
            miss(resolve_in(&fx, AppType::Codex, "ccs-kimi.kimi-k3")),
            StackMiss::Unknown
        );
        let message = StackMiss::Removed.message("ccs-claude.gone.g-1");
        assert!(message.contains("ccs-claude.gone.g-1"), "{message}");
    }

    #[test]
    fn stacked_ids_are_rejected_outside_stack_mode() {
        let fx = fixture();
        let id = "ccs-claude.kimi.kimi-k3";
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
            vec!["ccs-deepseek.deepseek-v4-pro"]
        );
        assert_eq!(
            hit(resolve_in(
                &fx,
                AppType::Codex,
                "ccs-deepseek.deepseek-v4-pro"
            )),
            (
                "ds".to_string(),
                "deepseek-v4-pro".to_string(),
                "ccs-deepseek.deepseek-v4-pro".to_string()
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
            (AppType::ClaudeDesktop, "ccs-claude.kimi.kimi-k3"),
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
            decode("G.", &AppType::Claude, "G.claude.cc.m"),
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
            decode("G.", &AppType::Codex, "G.cc.m"),
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
            decode("G.", &AppType::Claude, "G.claude.cc"),
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
            decode("G.", &AppType::Claude, "G.claude.default"),
            Decoded::Short { key: "default" }
        );
        assert_eq!(
            decode("G.", &AppType::Codex, "G.default"),
            Decoded::Short { key: "default" }
        );
    }

    #[test]
    fn empty_key_is_malformed() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.claude."),
            Decoded::Malformed
        );
        assert_eq!(decode("G.", &AppType::Codex, "G."), Decoded::Malformed);
    }

    #[test]
    fn one_m_marker_stripped_on_custom_prefix() {
        assert_eq!(
            decode("G.", &AppType::Claude, "G.claude.cc.m[1M]"),
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
        assert_eq!(decode("G.", &AppType::Codex, "ccs-k.m"), Decoded::Plain);
    }

    #[test]
    fn encode_roundtrip_custom_prefix() {
        assert_eq!(
            encode("G.", &AppType::Claude, "cc", "m", true),
            "G.claude.cc.m[1M]"
        );
        assert_eq!(encode("G.", &AppType::Codex, "cc", "m", false), "G.cc.m");
        assert_eq!(encode_short("G.", &AppType::Claude, "cc"), "G.claude.cc");
        assert_eq!(encode_short("G.", &AppType::Codex, "cc"), "G.cc");
        assert_eq!(
            encode_short("ccs-", &AppType::Claude, "cc"),
            "ccs-claude.cc"
        );
    }

    // fork Task 2: 短形式解析 → 成员默认模型

    #[test]
    fn short_form_resolves_member_default_model() {
        let fx = fixture();
        // 映射行：默认模型 = ANTHROPIC_MODEL（upstream 原值，1M 标记保留）
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude.kimi")),
            (
                "kimi".to_string(),
                "kimi-k3".to_string(),
                "ccs-claude.kimi".to_string()
            )
        );
        assert_eq!(
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude.zhipu")).1,
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
            hit(resolve_in(&fx, AppType::Claude, "ccs-claude.kimi")).1,
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
        let miss = miss(resolve_in(&fx, AppType::Claude, "ccs-claude.bare"));
        assert_eq!(miss, StackMiss::NoDefaultModel);
        let message = StackMiss::NoDefaultModel.message("ccs-claude.bare");
        assert!(message.contains("默认模型"), "{message}");
    }

    #[test]
    fn short_default_key_untouched_by_resolve() {
        // 保留 key default 的解绑语义在代理层（route_prefix::apply_session_routing），
        // resolve 不实现——钉住该边界
        let fx = fixture();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-claude.default")),
            StackMiss::Unknown
        );
    }

    #[test]
    fn short_form_removed_and_deleted_members_miss() {
        let fx = fixture();
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-claude.gone")),
            StackMiss::Removed
        );
        assert_eq!(
            miss(resolve_in(&fx, AppType::Claude, "ccs-claude.deleted")),
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
            miss(resolve_in(&fx, AppType::Claude, "ccs-claude.kimi")),
            StackMiss::StackOff
        );
    }

    // fork Task 5: 自定义 key（改名 + 墓碑）

    #[test]
    fn validate_member_key_matrix() {
        let mut stack = StackState::default();
        stack.keys.insert("kimi".to_string(), "kimi".to_string());
        stack.reserved_keys.insert("old".to_string());

        // 归一化：大小写保留、非法字符换横线、连续横线合并（doc/20261009-五项优化）
        assert_eq!(
            validate_member_key(&stack, "CommandCode"),
            Ok("CommandCode".to_string())
        );
        assert_eq!(validate_member_key(&stack, "a--b"), Ok("a-b".to_string()));
        assert_eq!(validate_member_key(&stack, "A_B"), Ok("A-B".to_string()));
        // `.` 是 id 的 key↔模型分隔符，归一化为横线
        assert_eq!(validate_member_key(&stack, "my.key"), Ok("my-key".to_string()));
        // 超长由 slug 截断到 24 位，不报错
        assert_eq!(
            validate_member_key(&stack, &"x".repeat(40)),
            Ok("x".repeat(24))
        );

        // 归一化后为空 / 保留字（忽略大小写——decode 端解绑判断本就忽略）/
        // 与登记簿或墓碑冲突（同样忽略大小写，避免视觉重复分组）
        assert!(validate_member_key(&stack, "").is_err());
        assert!(validate_member_key(&stack, "///").is_err());
        assert!(validate_member_key(&stack, "default").is_err());
        assert!(validate_member_key(&stack, "Default").is_err());
        assert!(validate_member_key(&stack, "Kimi").is_err());
        assert!(validate_member_key(&stack, "KIMI").is_err());
        assert!(validate_member_key(&stack, "old").is_err());
        assert!(validate_member_key(&stack, "Old").is_err());
    }

    #[test]
    fn auto_allocated_keys_stay_lowercase_even_amid_uppercase_ones() {
        // 手动改出的大写 key 占位后，自动分配同名小写 key 要顺延（去重忽略大小写），
        // 且自动分配本身永远产小写（与历史 key 风格一致）
        let mut stack = StackState::default();
        stack.keys.insert("Zhipu".to_string(), "zhipu".to_string());
        let provider = Provider::with_id("zhipu2".to_string(), "Zhipu".to_string(), json!({}), None);
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
}
