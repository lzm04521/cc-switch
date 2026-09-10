//! WorkBuddy MCP 同步与导入（`~/.workbuddy/mcp.json` 顶层 `mcpServers`）。
//!
//! WorkBuddy 的 MCP 容器与 CC Switch 统一 spec 同为 Claude 风格对象，
//! 同步/导入均直接透传，无需格式转换（对比 dsh 需要插件条目转换）。

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::error::AppError;
use serde_json::Value;
use std::collections::HashMap;

use super::validation::validate_server_spec;

/// 将单个 MCP 服务器同步到 workbuddy live 配置（写 `mcpServers.<id>`）
///
/// spec 已是 Claude 风格（统一格式），直接透传。
pub fn sync_single_server_to_workbuddy(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    crate::workbuddy_config::set_mcp_server(id, server_spec.clone())
}

/// 从 workbuddy live 配置移除单个 MCP 服务器
pub fn remove_server_from_workbuddy(id: &str) -> Result<(), AppError> {
    crate::workbuddy_config::remove_mcp_server(id)
}

/// Import MCP servers from WorkBuddy live config to unified structure
///
/// Existing servers will have WorkBuddy app enabled without overwriting other fields.
pub fn import_from_workbuddy(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let mcp_map = crate::workbuddy_config::get_mcp_servers()?;
    if mcp_map.is_empty() {
        return Ok(0);
    }

    // Ensure servers map exists
    let servers = config.mcp.servers.get_or_insert_with(HashMap::new);

    let mut changed = 0;
    for (id, spec) in mcp_map {
        // WorkBuddy spec 已是 Claude 风格，直接校验后透传
        if let Err(e) = validate_server_spec(&spec) {
            log::warn!("Skip invalid WorkBuddy MCP server '{id}': {e}");
            continue;
        }

        if let Some(existing) = servers.get_mut(&id) {
            // Existing server: just enable WorkBuddy app
            if !existing.apps.workbuddy {
                existing.apps.workbuddy = true;
                changed += 1;
                log::info!("MCP server '{id}' enabled for WorkBuddy");
            }
        } else {
            // New server: default to only WorkBuddy enabled
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
                        zcode: false,
                        dsh: false,
                        workbuddy: true,
                    },
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: Vec::new(),
                },
            );
            changed += 1;
            log::info!("Imported new MCP server '{id}' from WorkBuddy");
        }
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

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

    #[test]
    #[serial] // 与 workbuddy_config 测试共享 CC_SWITCH_TEST_HOME，必须串行
    fn import_from_workbuddy_imports_servers_and_enables_workbuddy_app() {
        let mut config = MultiAppConfig::default();
        config.mcp.servers = Some(HashMap::new());

        let id = "echo";
        let spec = json!({"type": "stdio", "command": "npx", "args": ["-y", "echo"]});

        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        crate::workbuddy_config::set_mcp_server(id, spec.clone()).expect("seed live server");

        let changed = import_from_workbuddy(&mut config).expect("import");
        assert_eq!(changed, 1);

        let servers = config.mcp.servers.as_ref().expect("servers map");
        let server = servers.get(id).expect("imported server");
        assert!(server.apps.workbuddy);
        assert!(!server.apps.claude);
        assert_eq!(server.server["command"], "npx");
    }
}
