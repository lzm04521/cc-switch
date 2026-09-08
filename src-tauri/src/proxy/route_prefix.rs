//! 会话级模型路由（前缀触发）模块
//!
//! 按请求 model 值的前缀（默认 `G.`，可配置）将请求路由到指定分组：
//! - `G.<key>`         → 路由到 key 分组，使用该分组默认模型（ANTHROPIC_MODEL）
//! - `G.<key>:<model>`  → 路由到 key 分组，显式模型透传（不落默认兜底）
//! - `<prefix>default`  → 解绑 session 粘性绑定，回落默认分组
//!
//! 设计文档：doc/20260908-会话级模型路由.md

use crate::proxy::model_mapper::strip_one_m_suffix_for_upstream;

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
}
