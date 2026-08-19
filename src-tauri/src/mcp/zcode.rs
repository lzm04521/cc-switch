//! ZCode MCP 同步和导入模块
//!
//! ZCode 的 `cli/config.json` 使用 Claude Code 风格的 MCP 容器
//! （`mcp.servers.<id> = { type, command, args, env, url, headers }`），
//! 与 CC Switch 统一 spec 内容兼容——同步时**直接透传**，无需格式转换
//! （对比 opencode 需要 `type: stdio↔local`、`command↔command 数组` 等转换）。

use std::collections::HashMap;

use serde_json::Value;

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::error::AppError;
use crate::zcode_config;

use super::validation::validate_server_spec;

/// 将单个 MCP 服务器同步到 zcode live 配置（写 `mcp.servers.<id>`）
///
/// spec 已是 Claude 风格（统一格式），直接透传。
pub fn sync_single_server_to_zcode(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    zcode_config::set_mcp_server(id, server_spec.clone())
}

/// 从 zcode live 配置移除单个 MCP 服务器
pub fn remove_server_from_zcode(id: &str) -> Result<(), AppError> {
    zcode_config::remove_mcp_server(id)
}

/// Import MCP servers from ZCode config to unified structure
///
/// Existing servers will have ZCode app enabled without overwriting other fields.
pub fn import_from_zcode(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let mcp_map = zcode_config::get_mcp_servers()?;
    if mcp_map.is_empty() {
        return Ok(0);
    }

    // Ensure servers map exists
    let servers = config.mcp.servers.get_or_insert_with(HashMap::new);

    let mut changed = 0;
    let mut errors = Vec::new();

    for (id, spec) in mcp_map {
        // ZCode spec 已是 Claude 风格，直接校验后透传
        if let Err(e) = validate_server_spec(&spec) {
            log::warn!("Skip invalid ZCode MCP server '{id}': {e}");
            errors.push(format!("{id}: {e}"));
            continue;
        }

        if let Some(existing) = servers.get_mut(&id) {
            // Existing server: just enable ZCode app
            if !existing.apps.zcode {
                existing.apps.zcode = true;
                changed += 1;
                log::info!("MCP server '{id}' enabled for ZCode");
            }
        } else {
            // New server: default to only ZCode enabled
            servers.insert(
                id.clone(),
                McpServer {
                    id: id.clone(),
                    name: id.clone(),
                    server: spec,
                    apps: McpApps {
                        claude: false,
                        codex: false,
                        gemini: false,
                        grokbuild: false,
                        opencode: false,
                        hermes: false,
                        zcode: true,
                        dsh: false,
                    },
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: Vec::new(),
                },
            );
            changed += 1;
            log::info!("Imported new MCP server '{id}' from ZCode");
        }
    }

    if !errors.is_empty() {
        log::warn!(
            "Import completed with {} failures: {:?}",
            errors.len(),
            errors
        );
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    #[test]
    #[serial] // 与 zcode_config 测试共享 CC_SWITCH_TEST_HOME，必须串行
    fn import_from_zcode_imports_servers_and_enables_zcode_app() {
        let mut config = MultiAppConfig::default();
        config.mcp.servers = Some(HashMap::new());

        let id = "echo";
        let spec = json!({"type": "stdio", "command": "npx", "args": ["-y", "echo"]});

        // 绕过 live 文件，直接测试 import 逻辑：手动构造
        // import_from_zcode 读 live，这里用临时文件验证
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        zcode_config::set_mcp_server(id, spec.clone()).expect("seed live server");

        let changed = import_from_zcode(&mut config).expect("import");
        assert_eq!(changed, 1);

        let servers = config.mcp.servers.as_ref().expect("servers map");
        let server = servers.get(id).expect("imported server");
        assert!(server.apps.zcode);
        assert!(!server.apps.claude);
        assert_eq!(server.server["command"], "npx");
    }

    struct TestHomeGuard(Option<std::ffi::OsString>);
    impl TestHomeGuard {
        fn set(home: &std::path::Path) -> Self {
            let guard = Self(std::env::var_os("CC_SWITCH_TEST_HOME"));
            std::env::set_var("CC_SWITCH_TEST_HOME", home);
            guard
        }
    }
    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }
}
