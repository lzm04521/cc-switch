//! ZCode 配置读写
//!
//! ZCode 是智谱 AI 的 AI 编程产品，底层基于 opencode 内核，配置根目录
//! `~/.zcode/`。本模块负责：
//!
//! - 路径解析：`~/.zcode/`（可通过设置-高级-配置文件目录覆盖）
//!   - `cli/config.json`：MCP / hooks / plugins（Claude Code 风格容器）
//!   - `cli/db/db.sqlite`：会话库（opencode schema）
//! - MCP 读写：`cli/config.json` 的 `mcp.servers.<id>` 段
//!
//! ZCode 的 provider（`~/.zcode/v2/config.json`）由 ZCode 应用内自管，
//! cc-switch 不读写。

use crate::config::write_json_file_with_contents;
use crate::error::AppError;
use crate::settings::get_zcode_override_dir;
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn zcode_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// 获取 ZCode 根目录（override 优先，否则 `~/.zcode/`）
pub fn get_zcode_dir() -> PathBuf {
    if let Some(override_dir) = get_zcode_override_dir() {
        return override_dir;
    }

    crate::config::get_home_dir().join(".zcode")
}

/// `~/.zcode/cli/config.json`（MCP / hooks / plugins）
pub fn get_zcode_cli_config_path() -> PathBuf {
    get_zcode_dir().join("cli").join("config.json")
}

/// `~/.zcode/cli/db/db.sqlite`（会话库，opencode schema）
pub fn get_zcode_db_path() -> PathBuf {
    get_zcode_dir().join("cli").join("db").join("db.sqlite")
}

fn read_cli_config_from_path(path: &Path) -> Result<Value, AppError> {
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
            "Failed to parse ZCode config: {}: {e}",
            path.display()
        ))
    })?;

    // 根节点必须是对象：下游对 `mcp.servers` 做索引赋值，数组/标量会 panic。
    // 与 opencode_config 一致：报错而不是重建根节点，避免覆盖用户自有配置。
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "ZCode 配置文件根节点必须是 JSON 对象: {}",
            path.display()
        )));
    }

    Ok(value)
}

fn write_cli_config_to_path(path: &Path, config: &Value) -> Result<(), AppError> {
    // 确保父目录存在（cli/ 可能尚未创建）
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    write_json_file_with_contents(path, config)?;
    log::debug!("ZCode config written to {path:?}");
    Ok(())
}

/// 读取 `mcp.servers` 段，返回 `IndexMap<String, Value>`（保留用户顺序）
pub fn get_mcp_servers() -> Result<IndexMap<String, Value>, AppError> {
    let config = read_cli_config_from_path(&get_zcode_cli_config_path())?;
    let servers = config
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .and_then(|servers| servers.as_object())
        .map(|map| {
            map.iter()
                .map(|(id, spec)| (id.clone(), spec.clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok(servers)
}

/// 写入或更新一个 MCP 服务器条目（写 `mcp.servers.<id>`）
///
/// 归一化：`mcp` / `mcp.servers` 非对象时重置为空对象，避免索引赋值
/// 静默失效或 panic（与 `opencode_config::set_mcp_server` 同口径）。
/// 用户自有的 hooks / plugins 等顶层字段原样保留。
pub fn set_mcp_server(id: &str, spec: Value) -> Result<(), AppError> {
    let _guard = zcode_config_lock().lock()?;
    let path = get_zcode_cli_config_path();
    let mut config = read_cli_config_from_path(&path)?;

    if !config.get("mcp").is_some_and(Value::is_object) {
        if config.get("mcp").is_some() {
            log::warn!("zcode cli/config.json 的 mcp 不是对象，已重置为空对象");
        }
        config["mcp"] = json!({});
    }
    let servers = config
        .get_mut("mcp")
        .and_then(|mcp| mcp.as_object_mut())
        .expect("mcp must be an object after normalization");
    if !servers.get("servers").is_some_and(Value::is_object) {
        if servers.contains_key("servers") {
            log::warn!("zcode cli/config.json 的 mcp.servers 不是对象，已重置为空对象");
        }
        servers.insert("servers".to_string(), json!({}));
    }
    servers
        .get_mut("servers")
        .and_then(|servers| servers.as_object_mut())
        .expect("mcp.servers must be an object after normalization")
        .insert(id.to_string(), spec);

    write_cli_config_to_path(&path, &config)
}

/// 删除一个 MCP 服务器条目（保留其他 server 与 hooks/plugins 等无关字段）
pub fn remove_mcp_server(id: &str) -> Result<(), AppError> {
    let _guard = zcode_config_lock().lock()?;
    let path = get_zcode_cli_config_path();
    let mut config = read_cli_config_from_path(&path)?;

    if let Some(servers) = config
        .get_mut("mcp")
        .and_then(|mcp| mcp.as_object_mut())
        .and_then(|mcp| mcp.get_mut("servers"))
        .and_then(|servers| servers.as_object_mut())
    {
        servers.remove(id);
    }

    write_cli_config_to_path(&path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    struct TestHomeGuard(Option<std::ffi::OsString>);
    impl TestHomeGuard {
        fn set(home: &Path) -> Self {
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
    fn set_mcp_server_writes_servers_section() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        set_mcp_server("echo", json!({"command": "npx"})).expect("set must succeed");

        let config =
            read_cli_config_from_path(&get_zcode_cli_config_path()).expect("reload");
        assert_eq!(config["mcp"]["servers"]["echo"]["command"], "npx");
    }

    #[test]
    #[serial]
    fn set_mcp_server_normalizes_non_object_sections() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        // `mcp` 是数组时旧代码的 as_object_mut 拿不到，写入会静默失效
        std::fs::create_dir_all(get_zcode_dir().join("cli")).expect("create cli dir");
        std::fs::write(
            get_zcode_cli_config_path(),
            r#"{"hooks": {"enabled": true}, "mcp": []}"#,
        )
        .expect("write config");

        set_mcp_server("echo", json!({"command": "npx"})).expect("set must succeed");

        let config =
            read_cli_config_from_path(&get_zcode_cli_config_path()).expect("reload");
        assert_eq!(
            config["mcp"]["servers"]["echo"]["command"], "npx",
            "server must actually be written"
        );
        assert_eq!(
            config["hooks"]["enabled"], true,
            "unrelated user config must be preserved"
        );
    }

    #[test]
    #[serial]
    fn remove_mcp_server_preserves_others_and_unrelated_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        set_mcp_server("keep", json!({"command": "keep-me"})).expect("set keep");
        set_mcp_server("drop", json!({"command": "drop-me"})).expect("set drop");
        // 模拟用户自有 hooks 配置
        let path = get_zcode_cli_config_path();
        let mut config = read_cli_config_from_path(&path).expect("reload");
        config["hooks"] = json!({"enabled": true});
        write_cli_config_to_path(&path, &config).expect("write hooks");

        remove_mcp_server("drop").expect("remove must succeed");

        let config = read_cli_config_from_path(&path).expect("reload");
        assert_eq!(config["mcp"]["servers"]["keep"]["command"], "keep-me");
        assert!(config["mcp"]["servers"].get("drop").is_none());
        assert_eq!(config["hooks"]["enabled"], true);
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
    fn get_zcode_dir_defaults_to_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());

        let dir = get_zcode_dir();
        assert!(dir.ends_with(".zcode"));
        assert_eq!(
            get_zcode_cli_config_path(),
            dir.join("cli").join("config.json")
        );
        assert_eq!(
            get_zcode_db_path(),
            dir.join("cli").join("db").join("db.sqlite")
        );
    }
}
