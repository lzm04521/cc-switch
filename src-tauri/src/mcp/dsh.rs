//! DSH MCP 同步和导入模块
//!
//! DSH 的 MCP server 以 `@deepseek-ai/dsh-mcp-client` 插件条目配置在
//! `<dsh home>/profiles/web/cordis.patch.yml`（由 dsh_mcp_config 读写）。
//! 同步时把 CC Switch 统一 spec（Claude 风格对象）转换为插件 config：
//! - stdio：`{type: "stdio"|省略, command, args, env, cwd}` ↔ `{transport: "stdio", ...}`
//! - http：`{type: "http", url, headers}` ↔ `{transport: "streamable-http", url, headers}`
//! - sse：DSH 不支持，同步时跳过并告警（不报错，单条能力缺失不阻塞其余条目）

use serde_json::{json, Map, Value};

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::dsh_mcp_config;
use crate::error::AppError;

use super::validation::validate_server_spec;

/// 仅在源对象存在且非 null 时拷贝字段
fn copy_if_present(dst: &mut Map<String, Value>, src: &Value, key: &str) {
    if let Some(v) = src.get(key) {
        if !v.is_null() {
            dst.insert(key.to_string(), v.clone());
        }
    }
}

/// dsh-mcp-client 的 serverName 契约：`[A-Za-z0-9_-]{1,32}`，
/// 同时是模型侧工具名 `mcp__<serverName>__*` 的命名空间
fn is_valid_dsh_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 统一 spec → dsh 插件 config（server 定义字段，serverName 由存储层填入）；
/// 不支持的 type（sse）返回 `Ok(None)`（调用方跳过）
fn unified_spec_to_dsh_config(id: &str, spec: &Value) -> Result<Option<Value>, AppError> {
    validate_server_spec(spec)?;
    if !is_valid_dsh_server_name(id) {
        return Err(AppError::McpValidation(format!(
            "DSH serverName 需匹配 [A-Za-z0-9_-]{{1,32}}：'{id}'"
        )));
    }
    let t = spec.get("type").and_then(Value::as_str).unwrap_or("stdio");
    match t {
        "stdio" => {
            let mut config = json!({"transport": "stdio"});
            let obj = config.as_object_mut().expect("just built object");
            copy_if_present(obj, spec, "command");
            copy_if_present(obj, spec, "args");
            copy_if_present(obj, spec, "env");
            copy_if_present(obj, spec, "cwd");
            Ok(Some(config))
        }
        "http" => {
            let mut config = json!({"transport": "streamable-http"});
            let obj = config.as_object_mut().expect("just built object");
            copy_if_present(obj, spec, "url");
            copy_if_present(obj, spec, "headers");
            Ok(Some(config))
        }
        other => {
            // sse：DSH 仅支持 stdio / streamable-http
            log::warn!(
                "Skip MCP server '{id}' for DSH: unsupported type '{other}' \
                 (dsh supports stdio/streamable-http only)"
            );
            Ok(None)
        }
    }
}

/// dsh 插件 config → 统一 spec（导入方向）；未知 transport 报错（由调用方 skip + 记录）
fn dsh_config_to_unified_spec(config: &Value) -> Result<Value, AppError> {
    let transport = config
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("stdio");
    let mut spec = match transport {
        "stdio" => json!({"type": "stdio"}),
        "streamable-http" => json!({"type": "http"}),
        other => {
            return Err(AppError::McpValidation(format!(
                "DSH MCP 条目使用了不支持的 transport: '{other}'（支持 stdio/streamable-http）"
            )));
        }
    };
    let obj = spec.as_object_mut().expect("just built object");
    if transport == "stdio" {
        copy_if_present(obj, config, "command");
        copy_if_present(obj, config, "args");
        copy_if_present(obj, config, "env");
        copy_if_present(obj, config, "cwd");
    } else {
        copy_if_present(obj, config, "url");
        copy_if_present(obj, config, "headers");
    }
    Ok(spec)
}

/// 将单个 MCP 服务器同步到 dsh live 配置（upsert cordis.patch.yml 插件条目）
pub fn sync_single_server_to_dsh(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    match unified_spec_to_dsh_config(id, server_spec)? {
        Some(config) => dsh_mcp_config::upsert_server(id, &config),
        None => Ok(()), // sse 等不支持类型已告警跳过
    }
}

/// 从 dsh live 配置移除单个 MCP 服务器
pub fn remove_server_from_dsh(id: &str) -> Result<(), AppError> {
    dsh_mcp_config::remove_server(id)
}

/// 从 dsh live 配置导入 MCP 服务器到统一结构
///
/// patch 中的 mcp-client 插件条目即"已启用"（DSH 侧禁用 = 条目不存在），
/// 导入条目全部置 `apps.dsh = true`。
pub fn import_from_dsh(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let entries = dsh_mcp_config::get_servers()?;
    if entries.is_empty() {
        return Ok(0);
    }

    let servers = config
        .mcp
        .servers
        .get_or_insert_with(std::collections::HashMap::new);

    let mut changed = 0;
    let mut errors = Vec::new();

    for (name, entry_config) in entries {
        let spec = match dsh_config_to_unified_spec(&entry_config) {
            Ok(spec) => spec,
            Err(e) => {
                log::warn!("Skip invalid DSH MCP server '{name}': {e}");
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        if let Err(e) = validate_server_spec(&spec) {
            log::warn!("Skip invalid DSH MCP server '{name}': {e}");
            errors.push(format!("{name}: {e}"));
            continue;
        }

        if let Some(existing) = servers.get_mut(&name) {
            if !existing.apps.dsh {
                existing.apps.dsh = true;
                changed += 1;
                log::info!("MCP server '{name}' enabled for DSH");
            }
        } else {
            servers.insert(
                name.clone(),
                McpServer {
                    id: name.clone(),
                    name: name.clone(),
                    server: spec,
                    apps: McpApps {
                        claude: false,
                        codex: false,
                        gemini: false,
                        grokbuild: false,
                        opencode: false,
                        hermes: false,
                        zcode: false,
                        dsh: true,
                    },
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: Vec::new(),
                },
            );
            changed += 1;
            log::info!("Imported new MCP server '{name}' from DSH");
        }
    }

    if !errors.is_empty() {
        log::warn!(
            "DSH import completed with {} failures: {:?}",
            errors.len(),
            errors
        );
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::MultiAppConfig;
    use serde_json::json;
    use serial_test::serial;

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
    fn stdio_spec_converts_to_dsh_config() {
        let config = unified_spec_to_dsh_config(
            "echo",
            &json!({"type": "stdio", "command": "npx", "args": ["-y", "echo"], "env": {"K": "V"}}),
        )
        .expect("convert")
        .expect("stdio supported");
        assert_eq!(config["transport"], "stdio");
        assert_eq!(config["command"], "npx");
        assert_eq!(config["args"][0], "-y");
        assert_eq!(config["env"]["K"], "V");
    }

    #[test]
    fn omitted_type_treated_as_stdio() {
        let config = unified_spec_to_dsh_config("e", &json!({"command": "node"}))
            .expect("convert")
            .expect("stdio");
        assert_eq!(config["transport"], "stdio");
    }

    #[test]
    fn http_spec_maps_to_streamable_http() {
        let config = unified_spec_to_dsh_config(
            "remote",
            &json!({"type": "http", "url": "https://example.com/mcp", "headers": {"A": "B"}}),
        )
        .expect("convert")
        .expect("http supported");
        assert_eq!(config["transport"], "streamable-http");
        assert_eq!(config["url"], "https://example.com/mcp");
        assert_eq!(config["headers"]["A"], "B");
    }

    #[test]
    fn sse_spec_is_skipped() {
        let result =
            unified_spec_to_dsh_config("old", &json!({"type": "sse", "url": "https://x/sse"}))
                .expect("no error");
        assert!(result.is_none(), "sse must be skipped, not error");
    }

    #[test]
    fn invalid_server_name_is_rejected() {
        let err = unified_spec_to_dsh_config(
            "bad name!",
            &json!({"type": "stdio", "command": "x"}),
        )
        .expect_err("spaces and '!' are outside the serverName contract");
        assert!(err.to_string().contains("serverName"));
    }

    #[test]
    fn dsh_config_converts_back_to_unified_spec() {
        let spec = dsh_config_to_unified_spec(&json!({
            "transport": "stdio", "command": "npx"
        }))
        .expect("convert");
        assert_eq!(spec["type"], "stdio");
        assert_eq!(spec["command"], "npx");

        let spec = dsh_config_to_unified_spec(&json!({
            "transport": "streamable-http", "url": "https://x/mcp"
        }))
        .expect("convert");
        assert_eq!(spec["type"], "http");
        assert_eq!(spec["url"], "https://x/mcp");
    }

    #[test]
    fn dsh_config_with_unknown_transport_is_error() {
        assert!(
            dsh_config_to_unified_spec(&json!({"transport": "websocket"})).is_err()
        );
    }

    #[test]
    #[serial]
    fn import_marks_servers_enabled_for_dsh() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        dsh_mcp_config::upsert_server(
            "on",
            &json!({"transport": "stdio", "command": "a"}),
        )
        .expect("seed on");

        let mut config = MultiAppConfig::default();
        let changed = import_from_dsh(&mut config).expect("import");
        assert_eq!(changed, 1);

        let servers = config.mcp.servers.expect("servers map");
        assert!(servers["on"].apps.dsh);
        assert!(!servers["on"].apps.claude);
    }

    #[test]
    #[serial]
    fn sync_and_remove_write_live_patch_yml() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        sync_single_server_to_dsh(
            &MultiAppConfig::default(),
            "echo",
            &json!({"type": "stdio", "command": "npx"}),
        )
        .expect("sync");
        let raw = std::fs::read_to_string(dsh_mcp_config::get_dsh_mcp_config_path())
            .expect("read patch");
        assert!(raw.contains("'@deepseek-ai/dsh-mcp-client'"));
        assert!(raw.contains("serverName: echo"));

        let servers = dsh_mcp_config::get_servers().expect("read");
        assert!(servers
            .iter()
            .any(|(n, c)| n == "echo" && c["transport"] == "stdio"));

        remove_server_from_dsh("echo").expect("remove");
        let servers = dsh_mcp_config::get_servers().expect("read");
        assert!(servers.iter().all(|(n, _)| n != "echo"));
    }

    #[test]
    #[serial]
    fn sync_sse_is_noop_not_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        sync_single_server_to_dsh(
            &MultiAppConfig::default(),
            "old",
            &json!({"type": "sse", "url": "https://x/sse"}),
        )
        .expect("skip without error");
        let servers = dsh_mcp_config::get_servers().expect("read");
        assert!(servers.is_empty());
    }
}
