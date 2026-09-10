//! WorkBuddy live 配置读写（`~/.workbuddy/mcp.json`）。
//!
//! cc-switch 仅管理 WorkBuddy 的 skills 部署与 MCP 配置；provider 由
//! WorkBuddy 应用内自管。MCP 配置为标准 Claude 风格：
//! 顶层 `{"mcpServers": {"<id>": {...}}}`。

use crate::config::write_json_file_with_contents;
use crate::error::AppError;
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn workbuddy_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 获取 WorkBuddy 根目录（固定 `~/.workbuddy`，无 override / 环境变量）
pub fn get_workbuddy_dir() -> PathBuf {
    crate::config::get_home_dir().join(".workbuddy")
}

/// `~/.workbuddy/mcp.json`
pub fn get_mcp_config_path() -> PathBuf {
    get_workbuddy_dir().join("mcp.json")
}

fn read_config_from_path(path: &Path) -> Result<Value, AppError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // 文件不存在视为空配置
            return Ok(json!({}));
        }
        Err(err) => return Err(AppError::io(path, err)),
    };
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse WorkBuddy config: {}: {e}",
            path.display()
        ))
    })?;

    // 根节点必须是对象：下游对 `mcpServers` 做索引赋值，数组/标量会 panic。
    // 与 zcode_config 一致：报错而不是重建根节点，避免覆盖用户自有配置。
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "WorkBuddy 配置文件根节点必须是 JSON 对象: {}",
            path.display()
        )));
    }

    Ok(value)
}

fn write_config_to_path(path: &Path, config: &Value) -> Result<(), AppError> {
    // 确保父目录存在（~/.workbuddy 可能尚未创建）
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    write_json_file_with_contents(path, config)?;
    log::debug!("WorkBuddy config written to {path:?}");
    Ok(())
}

/// 读取 `mcpServers` 段，返回 `IndexMap<String, Value>`（保留用户顺序）
pub fn get_mcp_servers() -> Result<IndexMap<String, Value>, AppError> {
    let config = read_config_from_path(&get_mcp_config_path())?;
    let servers = config
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(id, spec)| (id.clone(), spec.clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok(servers)
}

/// 写入或更新一个 MCP 服务器条目（写顶层 `mcpServers.<id>`）
///
/// 归一化：`mcpServers` 非对象时重置为空对象（与 `zcode_config::set_mcp_server`
/// 同口径）。用户自有的无关顶层字段原样保留。
pub fn set_mcp_server(id: &str, spec: Value) -> Result<(), AppError> {
    let _guard = workbuddy_config_lock().lock()?;
    let path = get_mcp_config_path();
    let mut config = read_config_from_path(&path)?;

    if !config.get("mcpServers").is_some_and(Value::is_object) {
        if config.get("mcpServers").is_some() {
            log::warn!("workbuddy mcp.json 的 mcpServers 不是对象，已重置为空对象");
        }
        config["mcpServers"] = json!({});
    }
    config
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .expect("mcpServers must be an object after normalization")
        .insert(id.to_string(), spec);

    write_config_to_path(&path, &config)
}

/// 删除一个 MCP 服务器条目（保留其他 server 与无关顶层字段）
pub fn remove_mcp_server(id: &str) -> Result<(), AppError> {
    let _guard = workbuddy_config_lock().lock()?;
    let path = get_mcp_config_path();
    let mut config = read_config_from_path(&path)?;

    if let Some(servers) = config.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove(id);
    }

    write_config_to_path(&path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[serial]
    fn get_mcp_servers_returns_empty_when_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        let servers = get_mcp_servers().expect("empty servers");
        assert!(servers.is_empty());
    }

    #[test]
    #[serial]
    fn set_mcp_server_writes_top_level_mcp_servers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        set_mcp_server("echo", json!({"type": "stdio", "command": "npx"}))
            .expect("set must succeed");

        let servers = get_mcp_servers().expect("reload");
        assert_eq!(servers["echo"]["command"], "npx");
    }

    #[test]
    #[serial]
    fn set_mcp_server_preserves_unrelated_fields_and_normalizes_section() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        // 预置用户自有无关字段 + 非对象的 mcpServers（数组）
        std::fs::create_dir_all(get_workbuddy_dir()).expect("mkdir");
        std::fs::write(
            get_mcp_config_path(),
            r#"{"someUserField": true, "mcpServers": []}"#,
        )
        .expect("seed config");

        set_mcp_server("echo", json!({"command": "npx"})).expect("set must succeed");

        let raw = std::fs::read_to_string(get_mcp_config_path()).expect("raw");
        let v: Value = serde_json::from_str(&raw).expect("parse");
        assert_eq!(v["someUserField"], true, "unrelated user field preserved");
        assert_eq!(v["mcpServers"]["echo"]["command"], "npx");
    }

    #[test]
    #[serial]
    fn remove_mcp_server_preserves_others() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        set_mcp_server("keep", json!({"command": "keep-me"})).expect("set keep");
        set_mcp_server("drop", json!({"command": "drop-me"})).expect("set drop");

        remove_mcp_server("drop").expect("remove must succeed");

        let servers = get_mcp_servers().expect("reload");
        assert_eq!(servers["keep"]["command"], "keep-me");
        assert!(servers.get("drop").is_none());
    }

    #[test]
    #[serial]
    fn non_object_root_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        std::fs::create_dir_all(get_workbuddy_dir()).expect("mkdir");
        std::fs::write(get_mcp_config_path(), r#"[1, 2]"#).expect("seed bad root");

        assert!(get_mcp_servers().is_err(), "array root must be rejected");
    }
}
