//! DSH MCP 配置读写（`<dsh home>/profiles/web/cordis.patch.yml`）
//!
//! DSH 不读取 `<dsh home>/mcp.json`（历史误设，已废弃）。MCP server 以
//! `@deepseek-ai/dsh-mcp-client` 插件条目的形式配置在 profile patch 层，
//! 每个 server 一条 insert：
//!
//! ```yaml
//! - insert:
//!     - id: mcp-<serverName>
//!       name: '@deepseek-ai/dsh-mcp-client'
//!       config:
//!         serverName: <serverName>
//!         transport: stdio | streamable-http
//!         command/args/env/cwd | url/headers
//! ```
//!
//! cc-switch 仅识别 name 为 dsh-mcp-client 的 insert 条目，其余 patch 条目
//! （插件启停、overrides 等）语义原样保留；写回按 DSH 原生风格重新序列化
//!（见 dump_patch_yaml——DSH Desktop 行级手术依赖其缩进契约）；首次读写时
//! 把旧版 mcp.json 的条目一次性迁移进 patch（原文件改名 `mcp.json.bak`，
//! 不删除）。根非数组、YAML 无法解析（含 `!!js` 表达式，round-trip 会破坏
//! 用户文件）时报错、不覆盖。

use crate::error::AppError;
use serde_json::{json, Map, Value as JsonValue};
use serde_yaml::Value as YamlValue;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// DSH profile 名：目前 DSH 仅有 `web` 一个 profile，目录固定为
/// `profiles/web/`；未来出现多 profile 时再引入设置项。
const DSH_PROFILE: &str = "web";
const MCP_CLIENT_PLUGIN: &str = "@deepseek-ai/dsh-mcp-client";

fn dsh_mcp_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// `<dsh home>/profiles/web/cordis.patch.yml`
pub fn get_dsh_mcp_config_path() -> PathBuf {
    crate::dsh_config::get_home()
        .join("profiles")
        .join(DSH_PROFILE)
        .join("cordis.patch.yml")
}

/// 旧版误设的存储位置，仅用于一次性迁移
fn legacy_mcp_json_path() -> PathBuf {
    crate::dsh_config::get_home().join("mcp.json")
}

// ===== YAML 装载 / 写回 =====

/// `!!js` 等 YAML 扩展表达式 serde_yaml 无法 round-trip（tag 会丢失），
/// 检测到即拒绝读写，避免静默破坏用户 patch 文件。
/// DSH 的扩展表达式只有 `!!js`（README 明示），且与 YAML 内置 tag 无前缀
/// 冲突，按子串检测即可覆盖行首/值位置两种写法。
/// 注释里的 `!!js` 字样不是扩展表达式——DSH 官方模板的头部注释就写着
/// "`!!js` expressions allowed"，按原文检测会误拒所有默认生成的文件。
fn contains_yaml_tag_extensions(content: &str) -> bool {
    content
        .lines()
        .any(|line| strip_yaml_comment(line).contains("!!js"))
}

/// 剥离行注释：整行注释返回空串，行内注释截断到 `#` 前（YAML 要求 `#`
/// 前有空白）。引号内的 ` #` 会被误剥，但引号内文本本就不是 tag，
/// round-trip 安全，漏检无害。
fn strip_yaml_comment(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        return "";
    }
    match line.find(" #") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// 提取文件头部注释块（首个非注释行之前的所有行），写回时原样保留。
/// 条目间与行内注释在 round-trip 中会丢失，仅头部说明得以幸存。
fn extract_header_comments(content: &str) -> String {
    let mut header = String::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            header.push_str(line);
            header.push('\n');
        } else {
            break;
        }
    }
    header
}

struct PatchFile {
    /// 顶层 patch 条目序列（序列化保插入序）
    entries: Vec<YamlValue>,
    /// 头部注释（写回时前置）
    header: String,
}

fn load_patch_file(path: &Path) -> Result<PatchFile, AppError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // 文件不存在视为空 patch（首次写入时创建）
            return Ok(PatchFile {
                entries: Vec::new(),
                header: String::new(),
            });
        }
        Err(err) => return Err(AppError::io(path, err)),
    };

    if contains_yaml_tag_extensions(&content) {
        return Err(AppError::Config(format!(
            "DSH cordis.patch.yml 含 !!js 等 YAML 扩展表达式，cc-switch 无法安全读写: {}",
            path.display()
        )));
    }

    let value: YamlValue = serde_yaml::from_str(&content).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse DSH cordis.patch.yml: {}: {e}",
            path.display()
        ))
    })?;

    // 根必须是数组（loader patch entry list）：其他形态属于用户损坏或异构
    // 文件，报错而不是重建根节点，避免覆盖用户自有配置
    let entries = match value {
        YamlValue::Sequence(items) => items,
        YamlValue::Null => Vec::new(),
        other => {
            return Err(AppError::Config(format!(
                "DSH cordis.patch.yml 根节点必须是数组: {}（实际为 {:?}）",
                path.display(),
                yaml_kind(&other)
            )))
        }
    };

    Ok(PatchFile {
        entries,
        header: extract_header_comments(&content),
    })
}

fn yaml_kind(value: &YamlValue) -> &'static str {
    match value {
        YamlValue::Null => "null",
        YamlValue::Bool(_) => "bool",
        YamlValue::Number(_) => "number",
        YamlValue::String(_) => "string",
        YamlValue::Sequence(_) => "sequence",
        YamlValue::Mapping(_) => "mapping",
        YamlValue::Tagged(_) => "tagged",
    }
}

// ===== DSH 原生风格序列化 =====
//
// 写回不能用 serde_yaml：它的 block 缩进是"序列项与父键对齐"（insert 内层
// 条目缩进 2），而 DSH Desktop 的行级手术（plugin-manager-patch.js 的
// topLevelEntryRe 等）以"顶层条目 0-2 空格、insert 内层条目 >=4 空格"区分
// 条目层级——2 空格内层条目会被误判为顶层条目，插件管理操作即误删整条
// 登记。DSH 自身所有写入方（js-yaml dump, indent 2）的输出契约是
// "序列项相对父键额外 +2 缩进"，cc-switch 必须产出同款风格（v3.20.0-4
// 事故分析见 doc/20260820-bug报告-DSH-patch-yml-序列化风格不兼容.md）。

/// 标量的行内文本形态；非标量返回 None
fn scalar_text(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::Null => Some("null".to_string()),
        YamlValue::Bool(b) => Some(b.to_string()),
        YamlValue::Number(n) => Some(n.to_string()),
        YamlValue::String(s) => Some(quote_dsh_style(s)),
        _ => None,
    }
}

/// 裸标量白名单：仅 ASCII 字母数字与 `_ ./\:=+-*,@`，首字符额外排除
/// `@`（YAML 保留字）、`.` 以外的特殊形态；可解析为 bool/null/数字的
/// 字面量与文档标记（---/...）必须引号。引号形态永远语义安全，
/// 白名单偏保守只是让常见路径/URL 保持 DSH 原生的裸风格。
fn is_bare_safe(s: &str) -> bool {
    if s.is_empty() || s.ends_with(':') {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" | "inf" | "nan"
    ) {
        return false;
    }
    if s.parse::<f64>().is_ok() {
        return false;
    }
    let first = s.chars().next().unwrap_or(' ');
    if !(first.is_ascii_alphanumeric() || matches!(first, '_' | '*' | '.' | '-')) {
        return false;
    }
    if matches!(s, "-" | "--" | "---" | "...") {
        return false;
    }
    s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '_' | '.' | '/' | '\\' | ':' | '=' | '-' | '+' | '*' | ',' | '@'
            )
    })
}

/// 双引号 + YAML/js-yaml 兼容转义（\x 覆盖控制字符，非 ASCII 原样输出）
fn double_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn quote_dsh_style(s: &str) -> String {
    if is_bare_safe(s) {
        s.to_string()
    } else {
        double_quote(s)
    }
}

/// 序列块：每项 `<indent>- ` + 条目体（内容基线 indent+2）。
/// 值为序列时项相对父键 +2 缩进——DSH 行级手术的层级契约所在。
fn emit_sequence(out: &mut String, items: &[YamlValue], indent: usize) -> Result<(), AppError> {
    for item in items {
        out.push_str(&" ".repeat(indent));
        out.push_str("- ");
        emit_entry_body(out, item, indent + 2)?;
    }
    Ok(())
}

/// 条目体（紧跟 `- ` 或行首）：标量同行收行；mapping 首键同行、其余键
/// 按基线缩进；子序列换行后项再进 2
fn emit_entry_body(out: &mut String, value: &YamlValue, base: usize) -> Result<(), AppError> {
    if let Some(text) = scalar_text(value) {
        out.push_str(&text);
        out.push('\n');
        return Ok(());
    }
    match value {
        YamlValue::Mapping(map) => {
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(&" ".repeat(base));
                }
                emit_mapping_entry(out, k, v, base)?;
            }
            Ok(())
        }
        YamlValue::Sequence(items) => {
            out.push('\n');
            emit_sequence(out, items, base + 2)
        }
        YamlValue::Tagged(_) => Err(AppError::Config(
            "DSH cordis.patch.yml 含 !!js 扩展表达式条目，cc-switch 不重写该文件".into(),
        )),
        // 标量已在函数开头处理
        _ => unreachable!("scalar handled above"),
    }
}

/// mapping 键值行：标量值同行；空集合 inline（[] / {}）；非空子结构
/// 换行后整体缩进 +2（序列项再 +2，见 emit_sequence）
fn emit_mapping_entry(
    out: &mut String,
    key: &YamlValue,
    value: &YamlValue,
    base: usize,
) -> Result<(), AppError> {
    let Some(key_text) = scalar_text(key) else {
        return Err(AppError::Config(
            "DSH cordis.patch.yml 存在非字符串键，cc-switch 无法安全序列化".into(),
        ));
    };
    if let Some(text) = scalar_text(value) {
        out.push_str(&key_text);
        out.push_str(": ");
        out.push_str(&text);
        out.push('\n');
        return Ok(());
    }
    match value {
        YamlValue::Sequence(items) if items.is_empty() => {
            out.push_str(&key_text);
            out.push_str(": []\n");
        }
        YamlValue::Sequence(items) => {
            out.push_str(&key_text);
            out.push_str(":\n");
            emit_sequence(out, items, base + 2)?;
        }
        YamlValue::Mapping(map) if map.is_empty() => {
            out.push_str(&key_text);
            out.push_str(": {}\n");
        }
        YamlValue::Mapping(map) => {
            out.push_str(&key_text);
            out.push_str(":\n");
            for (k, v) in map {
                out.push_str(&" ".repeat(base + 2));
                emit_mapping_entry(out, k, v, base + 2)?;
            }
        }
        YamlValue::Tagged(_) => {
            return Err(AppError::Config(
                "DSH cordis.patch.yml 含 !!js 扩展表达式条目，cc-switch 不重写该文件".into(),
            ))
        }
        _ => unreachable!("scalar handled above"),
    }
    Ok(())
}

/// 整体序列化：与 DSH 写入方（js-yaml dump, indent 2）同风格；空条目
/// 输出 `[]` 占位（DSH 约定 patch 文件保持合法顶层数组）
fn dump_patch_yaml(entries: &[YamlValue]) -> Result<String, AppError> {
    if entries.is_empty() {
        return Ok("[]\n".to_string());
    }
    let mut out = String::new();
    emit_sequence(&mut out, entries, 0)?;
    Ok(out)
}

fn write_patch_file(path: &Path, file: &PatchFile) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    let body = dump_patch_yaml(&file.entries)?;
    let mut out = String::new();
    if !file.header.is_empty() {
        out.push_str(&file.header);
    }
    out.push_str(&body);
    std::fs::write(path, out).map_err(|e| AppError::io(path, e))?;
    log::debug!("DSH cordis.patch.yml written to {:?}", path.display());
    Ok(())
}

// ===== mcp-client 插件条目定位 =====

/// 遍历所有 patch 条目的 insert 数组，产出 (insert 所在 patch 下标,
/// 插件条目在 insert 内的下标, serverName)。结构不符的条目静默跳过
/// （读写均不触碰它们）。
fn find_mcp_client_entries(entries: &[YamlValue]) -> Vec<(usize, usize, String)> {
    let mut found = Vec::new();
    for (patch_idx, entry) in entries.iter().enumerate() {
        let Some(insert) = entry.as_mapping().and_then(|m| m.get("insert")) else {
            continue;
        };
        let Some(items) = insert.as_sequence() else {
            continue;
        };
        for (item_idx, item) in items.iter().enumerate() {
            let Some(plugin_name) = item.as_mapping().and_then(|m| m.get("name")) else {
                continue;
            };
            if plugin_name.as_str() != Some(MCP_CLIENT_PLUGIN) {
                continue;
            }
            let server_name = item
                .as_mapping()
                .and_then(|m| m.get("config"))
                .and_then(|c| c.as_mapping())
                .and_then(|c| c.get("serverName"))
                .and_then(|n| n.as_str().map(str::to_string));
            if let Some(server_name) = server_name {
                found.push((patch_idx, item_idx, server_name));
            }
        }
    }
    found
}

fn json_to_yaml(value: &JsonValue) -> Result<YamlValue, AppError> {
    serde_yaml::to_value(value)
        .map_err(|e| AppError::Config(format!("Failed to convert JSON to YAML value: {e}")))
}

fn yaml_to_json(value: &YamlValue) -> Result<JsonValue, AppError> {
    serde_json::to_value(value)
        .map_err(|e| AppError::Config(format!("Failed to convert YAML to JSON value: {e}")))
}

/// 组装一条独立的 `- insert: [plugin]` patch 条目（与 DSH 生成文件中
/// 每插件一条 insert 的风格一致）。config 须已含首位的 serverName。
fn make_insert_entry(server_name: &str, config: &JsonValue) -> Result<YamlValue, AppError> {
    let plugin = json!({
        "id": format!("mcp-{server_name}"),
        "name": MCP_CLIENT_PLUGIN,
        "config": config,
    });
    let insert = json!({ "insert": [plugin] });
    json_to_yaml(&insert)
}

/// 从插件条目提取 config 对象并剥离 serverName（对外只暴露 server 定义字段）
fn plugin_config_to_json(plugin: &YamlValue) -> Option<JsonValue> {
    let config = plugin.as_mapping()?.get("config")?;
    let mut json_config = yaml_to_json(config).ok()?;
    if let Some(obj) = json_config.as_object_mut() {
        obj.remove("serverName");
    }
    Some(json_config)
}

// ===== 旧版 mcp.json 一次性迁移 =====

/// patch 中尚无 mcp-client 条目且旧 mcp.json 存在时，把其条目迁移为
/// insert 插件条目。成功后 mcp.json 改名 `mcp.json.bak` 留档；
/// 返回是否产生了需要写回的变更。
fn migrate_legacy_mcp_json_if_needed(entries: &mut Vec<YamlValue>) -> bool {
    let legacy_path = legacy_mcp_json_path();
    if !legacy_path.is_file() {
        return false;
    }
    // 已有 mcp 条目说明迁移完成或用户已手工配置，不重复迁移
    if !find_mcp_client_entries(entries).is_empty() {
        return false;
    }

    let content = match std::fs::read_to_string(&legacy_path) {
        Ok(content) => content,
        Err(err) => {
            log::warn!("读取旧版 DSH mcp.json 失败（跳过迁移）: {err}");
            return false;
        }
    };
    let legacy: JsonValue = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => {
            log::warn!("旧版 DSH mcp.json 无法解析（跳过迁移）: {err}");
            return false;
        }
    };
    let Some(items) = legacy.get("servers").and_then(|v| v.as_array()) else {
        log::warn!("旧版 DSH mcp.json 缺少 servers 数组（跳过迁移）");
        return false;
    };

    let mut migrated = 0usize;
    for item in items {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            log::warn!("旧版 DSH mcp.json 存在缺少 name 的条目，跳过: {item}");
            continue;
        };
        if item.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            log::info!("旧版 DSH mcp.json 条目 '{name}' 为 enabled:false，不迁移");
            continue;
        }
        // serverName 首位 + 白名单字段直拷（transport/command/args/env/cwd/url/headers）
        let mut config = Map::new();
        config.insert("serverName".to_string(), json!(name));
        for key in [
            "transport",
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
        ] {
            if let Some(value) = item.get(key) {
                if !value.is_null() {
                    config.insert(key.to_string(), value.clone());
                }
            }
        }
        match make_insert_entry(name, &JsonValue::Object(config)) {
            Ok(entry) => {
                entries.push(entry);
                migrated += 1;
            }
            Err(err) => log::warn!("迁移 DSH MCP 条目 '{name}' 失败，跳过: {err}"),
        }
    }

    // 留档原文件：迁移有产出时改名；无产出（全部跳过）也改名，避免每次
    // 读写都重试迁移一个不可用的文件。rename 失败仅告警——条目已迁移，
    // 下次会因"已有 mcp 条目"而跳过，不会重复。
    let backup = legacy_mcp_json_path().with_extension("json.bak");
    if let Err(err) = std::fs::rename(&legacy_path, &backup) {
        log::warn!(
            "旧版 DSH mcp.json 改名失败（不影响迁移结果）: {} -> {}: {err}",
            legacy_path.display(),
            backup.display()
        );
    } else {
        log::info!(
            "已把旧版 DSH mcp.json 的 {migrated} 个条目迁移到 cordis.patch.yml，原文件留档为 {}",
            backup.display()
        );
    }
    migrated > 0
}

/// 装载 patch 并完成惰性迁移；迁移产生变更时先写回再返回
fn load_and_migrate(path: &Path) -> Result<PatchFile, AppError> {
    let mut file = load_patch_file(path)?;
    if migrate_legacy_mcp_json_if_needed(&mut file.entries) {
        write_patch_file(path, &file)?;
    }
    Ok(file)
}

// ===== 公开 API（供 mcp/dsh.rs 调用） =====

/// 读取全部 MCP server，返回 `(serverName, config)` 列表（保留文件顺序）。
/// config 为插件 config 去掉 serverName 后的 server 定义。
pub fn get_servers() -> Result<Vec<(String, JsonValue)>, AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let path = get_dsh_mcp_config_path();
    let file = load_and_migrate(&path)?;
    let mut result = Vec::new();
    for (patch_idx, item_idx, server_name) in find_mcp_client_entries(&file.entries) {
        let plugin = file.entries[patch_idx]
            .as_mapping()
            .and_then(|m| m.get("insert"))
            .and_then(|v| v.as_sequence())
            .and_then(|s| s.get(item_idx));
        let config = plugin
            .and_then(plugin_config_to_json)
            .unwrap_or_else(|| json!({}));
        result.push((server_name, config));
    }
    Ok(result)
}

/// 按 serverName 写入/更新 MCP server；config 为 server 定义字段
/// （transport/command/args/env/cwd/url/headers）。同名替换（保留插件条目
/// 原位置与用户自定义 id），异名在 patch 末尾追加一条独立 insert。
pub fn upsert_server(name: &str, config: &JsonValue) -> Result<(), AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let path = get_dsh_mcp_config_path();
    let mut file = load_and_migrate(&path)?;

    // 组装含 serverName 的完整 config
    let mut full_config = Map::new();
    full_config.insert("serverName".to_string(), json!(name));
    if let Some(source) = config.as_object() {
        for (key, value) in source {
            full_config.insert(key.clone(), value.clone());
        }
    }
    let full_config = JsonValue::Object(full_config);

    if let Some((patch_idx, item_idx, _)) = find_mcp_client_entries(&file.entries)
        .into_iter()
        .find(|(_, _, server_name)| server_name == name)
    {
        // 原位替换：保留用户自定义的插件 id（缺省则补 mcp-<name>）
        let insert_seq = file.entries[patch_idx]
            .as_mapping_mut()
            .and_then(|m| m.get_mut("insert"))
            .and_then(|v| v.as_sequence_mut());
        let Some(insert_seq) = insert_seq else {
            return Err(AppError::Config(
                "DSH cordis.patch.yml 的 insert 结构损坏，无法更新条目".into(),
            ));
        };
        let old_id = insert_seq[item_idx]
            .as_mapping()
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let plugin = json!({
            "id": old_id.unwrap_or_else(|| format!("mcp-{name}")),
            "name": MCP_CLIENT_PLUGIN,
            "config": full_config,
        });
        insert_seq[item_idx] = json_to_yaml(&plugin)?;
    } else {
        file.entries.push(make_insert_entry(name, &full_config)?);
    }
    write_patch_file(&path, &file)
}

/// 按 serverName 移除 MCP server（其余 patch 条目与同 insert 内的其他
/// 插件条目原样保留；insert 被清空时移除该空 patch 条目）
pub fn remove_server(name: &str) -> Result<(), AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let path = get_dsh_mcp_config_path();
    let mut file = load_and_migrate(&path)?;

    let targets: Vec<(usize, usize)> = find_mcp_client_entries(&file.entries)
        .into_iter()
        .filter(|(_, _, server_name)| server_name == name)
        .map(|(p, i, _)| (p, i))
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    // 同一 insert 内可能命中多条，倒序按 (patch_idx, item_idx) 删除
    for (patch_idx, item_idx) in targets.into_iter().rev() {
        let is_empty_after = {
            let insert_seq = file.entries[patch_idx]
                .as_mapping_mut()
                .and_then(|m| m.get_mut("insert"))
                .and_then(|v| v.as_sequence_mut());
            if let Some(insert_seq) = insert_seq {
                insert_seq.remove(item_idx);
                insert_seq.is_empty()
            } else {
                false
            }
        };
        if is_empty_after {
            file.entries.remove(patch_idx);
        }
    }
    write_patch_file(&path, &file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    /// 同时 guard CC_SWITCH_TEST_HOME 与 DSH_HOME，
    /// 避免 DSH_HOME 泄漏到其他测试或受本机环境影响
    /// （本机 ~/.dsh 有真实数据）。
    struct TestEnvGuard {
        home: Option<std::ffi::OsString>,
        dsh_home: Option<std::ffi::OsString>,
    }
    impl TestEnvGuard {
        fn set(home: &std::path::Path) -> Self {
            let guard = Self {
                home: std::env::var_os("CC_SWITCH_TEST_HOME"),
                dsh_home: std::env::var_os("DSH_HOME"),
            };
            std::env::set_var("CC_SWITCH_TEST_HOME", home);
            std::env::remove_var("DSH_HOME");
            guard
        }
    }
    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match self.home.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
            match self.dsh_home.take() {
                Some(value) => std::env::set_var("DSH_HOME", value),
                None => std::env::remove_var("DSH_HOME"),
            }
        }
    }

    #[test]
    #[serial]
    fn mcp_path_points_to_profile_patch_yml() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        assert_eq!(
            path.file_name().map(std::ffi::OsStr::to_str),
            Some(Some("cordis.patch.yml"))
        );
        // <home>/profiles/web/cordis.patch.yml
        assert!(path
            .parent()
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "web")));
        assert!(path
            .parent()
            .and_then(Path::parent)
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "profiles")));
    }

    #[test]
    #[serial]
    fn get_servers_empty_when_file_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let servers = get_servers().expect("empty servers");
        assert!(servers.is_empty());
        // 读操作不创建文件
        assert!(!get_dsh_mcp_config_path().exists());
    }

    #[test]
    #[serial]
    fn upsert_roundtrip_preserves_foreign_entries_and_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        // 预置用户自有 patch 内容：头部注释 + 无关插件条目 + 无关 mcp-client 配置
        std::fs::write(
            &path,
            "# Your patch layer for this dsh profile\n# second header line\n- id: harness-pet\n  disabled: true\n- insert:\n    - id: balance\n      name: '@deepseek-ai/dsh-balance'\n",
        )
        .expect("seed patch");

        upsert_server(
            "echo",
            &json!({"transport": "stdio", "command": "npx", "args": ["-y", "echo"]}),
        )
        .expect("upsert");

        let servers = get_servers().expect("read");
        assert_eq!(servers.len(), 1);
        let (name, config) = &servers[0];
        assert_eq!(name, "echo");
        assert_eq!(config["transport"], "stdio");
        assert_eq!(config["command"], "npx");
        // serverName 不暴露给调用方
        assert!(config.get("serverName").is_none());

        let raw = std::fs::read_to_string(&path).expect("raw read");
        assert!(
            raw.starts_with("# Your patch layer for this dsh profile\n# second header line\n"),
            "header comments preserved"
        );
        assert!(raw.contains("harness-pet"), "foreign patch entry preserved");
        assert!(raw.contains("\"@deepseek-ai/dsh-balance\""));
        assert!(raw.contains("\"@deepseek-ai/dsh-mcp-client\""));
        assert!(raw.contains("serverName: echo"));
        // DSH 缩进契约：insert 内层条目 4 空格，绝不允许 2 空格 `- id:`
        // （DSH Desktop 行级手术按 0-2/>=4 区分顶层与内层条目）
        assert!(
            raw.contains("\n    - id: "),
            "insert inner entries at 4 spaces"
        );
        assert!(
            !raw.contains("\n  - "),
            "no 2-space sequence items (DSH contract)"
        );

        // 同名再写 = 更新而非追加
        upsert_server("echo", &json!({"transport": "stdio", "command": "updated"}))
            .expect("upsert again");
        let servers = get_servers().expect("read");
        assert_eq!(servers.len(), 1, "same-name upsert replaces, not appends");
        assert_eq!(servers[0].1["command"], "updated");
    }

    #[test]
    #[serial]
    fn remove_server_preserves_others_and_drops_empty_insert() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        upsert_server("keep", &json!({"transport": "stdio", "command": "k"})).expect("seed keep");
        upsert_server(
            "drop",
            &json!({"transport": "streamable-http", "url": "http://x/mcp"}),
        )
        .expect("seed drop");

        remove_server("drop").expect("remove");

        let servers = get_servers().expect("read");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].0, "keep");

        let raw = std::fs::read_to_string(get_dsh_mcp_config_path()).expect("raw");
        assert!(!raw.contains("drop"), "empty insert entry removed entirely");
        assert!(raw.contains("keep"));
    }

    #[test]
    #[serial]
    fn root_not_sequence_is_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "mapping: true\n").expect("seed mapping root");

        assert!(get_servers().is_err(), "mapping root must be rejected");
        assert!(
            upsert_server("x", &json!({"transport": "stdio"})).is_err(),
            "must not overwrite user file with rebuilt root"
        );
    }

    #[test]
    #[serial]
    fn js_tag_expressions_are_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            "- insert:\n    - id: mcp-x\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: x\n        env:\n          TOKEN: !!js process.env.TOKEN\n",
        )
        .expect("seed js-tag patch");

        assert!(get_servers().is_err(), "!!js must be rejected, not parsed");
        let raw = std::fs::read_to_string(&path).expect("file untouched");
        assert!(raw.contains("!!js"), "rejected file must not be rewritten");
    }

    #[test]
    #[serial]
    fn js_tag_in_comments_is_allowed() {
        // DSH 官方模板头部注释含 "`!!js` expressions allowed"，行内注释
        // 同理——只有真实的值位置扩展表达式才应拒绝
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            "# Your patch layer for this dsh profile, applied after every bundle layer:\n# a top-level YAML array of loader patch entries (id-targeted config\n# overrides, disables, and insert lists; `!!js` expressions allowed).\n- id: harness-pet\n  disabled: true # inline note about !!js\n",
        )
        .expect("seed official template");

        let servers = get_servers().expect("comments mentioning !!js must be readable");
        assert!(servers.is_empty());

        upsert_server("echo", &json!({"transport": "stdio", "command": "npx"})).expect("upsert");
        let raw = std::fs::read_to_string(&path).expect("raw");
        assert!(
            raw.contains("`!!js` expressions allowed"),
            "official header preserved"
        );
        assert!(raw.contains("harness-pet"), "foreign entry preserved");
        assert!(raw.contains("serverName: echo"));
    }

    #[test]
    #[serial]
    fn dump_matches_dsh_yaml_style() {
        // 纯函数：与 DSH 写入方（js-yaml dump, indent 2）的关键风格对齐
        let entries: Vec<YamlValue> = serde_yaml::from_str(
            "[{insert: [{id: mcp-x, name: \"@deepseek-ai/dsh-mcp-client\", \
             config: {serverName: x, transport: stdio, command: npx, \
             args: ['@playwright/mcp@latest', '600', 'a b'], timeout: 600, \
             args2: [], url: 'https://x/y', flag: true}}]}, \
             {id: harness-pet, disabled: true}]",
        )
        .expect("seed yaml");
        let out = dump_patch_yaml(&entries).expect("dump");
        // insert 内层条目 4 空格；config 键 6/8 空格
        assert!(out.contains("\n    - id: mcp-x\n"));
        assert!(out.contains("\n      name: \"@deepseek-ai/dsh-mcp-client\"\n"));
        assert!(out.contains("\n        serverName: x\n"));
        // 嵌套序列项 = 键缩进 +2（10 空格）；数字字符串加引号、含空格串加引号
        assert!(out.contains("\n          - \"@playwright/mcp@latest\"\n"));
        assert!(out.contains("\n          - \"600\"\n"));
        assert!(out.contains("\n          - \"a b\"\n"));
        // 数字 / bool / URL 裸写；空序列 inline
        assert!(out.contains("timeout: 600\n"));
        assert!(out.contains("flag: true\n"));
        assert!(out.contains("url: https://x/y\n"));
        assert!(out.contains("args2: []\n"));
        // 顶层直条目 0 缩进
        assert!(out.ends_with("- id: harness-pet\n  disabled: true\n"));
        // 契约红线：全文不允许 2 空格缩进的序列项
        assert!(!out.contains("\n  - "));
        // 语义 round-trip 等价
        let parsed: Vec<YamlValue> = serde_yaml::from_str(&out).expect("re-parse");
        let orig: String = serde_yaml::to_string(&entries).expect("orig");
        let reparsed: String = serde_yaml::to_string(&parsed).expect("re-dump");
        assert_eq!(orig, reparsed, "dump output must be semantically identical");
        // 空条目输出 [] 占位
        assert_eq!(dump_patch_yaml(&[]).unwrap(), "[]\n");
    }

    #[test]
    #[serial]
    fn upsert_keeps_dsh_style_on_real_world_file() {
        // 以 DSH 官方风格的真实样例（头部注释 + 无关条目 + 复杂 MCP 条目）
        // 为底，upsert 后文件必须仍满足 DSH 缩进契约且既有条目语义不变
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            "# Your patch layer for this dsh profile:\n# overrides, disables, and insert lists; `!!js` expressions allowed).\n\
             - id: harness-pet\n  disabled: true\n\
             - insert:\n    - id: mcp-windbg\n      name: \"@deepseek-ai/dsh-mcp-client\"\n      config:\n        serverName: mcp_windbg\n        transport: stdio\n        command: python\n        args:\n          - -m\n          - mcp_windbg\n          - --timeout\n          - \"600\"\n        env:\n          _NT_SYMBOL_PATH: SRV*V:\\dbg-symbols*https://msdl.microsoft.com/download/symbols\n",
        )
        .expect("seed dsh-style patch");

        upsert_server(
            "echo",
            &json!({"transport": "stdio", "command": "npx", "args": ["-y", "echo"]}),
        )
        .expect("upsert");

        let raw = std::fs::read_to_string(&path).expect("raw");
        // 契约红线：不允许任何 2 空格缩进序列项（顶层条目 0、insert 内层 4）
        assert!(!raw.contains("\n  - "), "DSH indent contract violated");
        // 既有条目按 DSH 风格保留：4 空格内层、10 空格 args 项、引号数字串
        assert!(raw.contains("\n    - id: mcp-windbg\n"));
        assert!(raw.contains("\n          - mcp_windbg\n"));
        assert!(raw.contains("\n          - \"600\"\n"));
        assert!(raw.contains("_NT_SYMBOL_PATH: SRV*V:\\dbg-symbols"));
        // 新增条目同为 DSH 风格
        assert!(raw.contains("\n    - id: mcp-echo\n"));
        assert!(raw.contains("serverName: echo"));
        // 读回语义完整：两个 server 均在
        let servers = get_servers().expect("read back");
        let names: Vec<&str> = servers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["mcp_windbg", "echo"]);
        assert_eq!(servers[0].1["args"][3], "600");
    }

    #[test]
    #[serial]
    fn legacy_mcp_json_is_migrated_once_with_backup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        // 预置旧版 mcp.json：两条有效 + 一条 enabled:false + 一条缺 name
        std::fs::create_dir_all(legacy_mcp_json_path().parent().expect("parent"))
            .expect("mkdir dsh home");
        std::fs::write(
            legacy_mcp_json_path(),
            r#"{"servers": [
                {"name": "echo", "transport": "stdio", "command": "npx", "args": ["-y"]},
                {"name": "remote", "transport": "streamable-http", "url": "http://x/mcp"},
                {"name": "off", "transport": "stdio", "command": "x", "enabled": false},
                {"transport": "stdio", "command": "nameless"}
            ]}"#,
        )
        .expect("seed legacy");

        let servers = get_servers().expect("read triggers migration");
        let names: Vec<&str> = servers.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["echo", "remote"],
            "enabled:false and nameless skipped"
        );

        let echo_config = &servers[0].1;
        assert_eq!(echo_config["transport"], "stdio");
        assert_eq!(echo_config["command"], "npx");
        assert_eq!(echo_config["args"][0], "-y");

        // 原文件留档为 .bak，旧文件不再存在
        assert!(!legacy_mcp_json_path().exists());
        assert!(legacy_mcp_json_path().with_extension("json.bak").exists());

        // 迁移后的 patch 可继续 upsert/remove，且不会重复迁移
        upsert_server("extra", &json!({"transport": "stdio", "command": "e"}))
            .expect("upsert after migration");
        let servers = get_servers().expect("read");
        assert_eq!(servers.len(), 3);
    }

    #[test]
    #[serial]
    fn upsert_replaces_in_place_keeping_custom_plugin_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        // 用户手工配置的 mcp-client 条目，自定义插件 id
        std::fs::write(
            &path,
            "- insert:\n    - id: my-custom-id\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: echo\n        transport: stdio\n        command: old\n",
        )
        .expect("seed custom");

        upsert_server("echo", &json!({"transport": "stdio", "command": "new"})).expect("upsert");

        let raw = std::fs::read_to_string(&path).expect("raw");
        assert!(raw.contains("my-custom-id"), "custom plugin id preserved");
        assert!(raw.contains("command: new"));
        assert!(!raw.contains("mcp-echo"), "default id not introduced");
        let servers = get_servers().expect("read");
        assert_eq!(servers.len(), 1, "replaced in place, not appended");
    }
}
