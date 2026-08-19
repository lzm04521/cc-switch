//! DSH (DeepSeek Harness) mcp.json 读写
//!
//! DSH 的 MCP 配置位于 `<dsh home>/mcp.json`（home 解析复用 dsh_config::get_home：
//! 设置覆盖 → DSH_HOME 环境变量 → ~/.dsh），结构为 `{"servers": [...]}` 数组容器，
//! 条目按 `name` 字段定位。cc-switch 不认识的条目与顶层字段原样保留，
//! 根非对象或 servers 非数组时报错、不覆盖用户文件。

use crate::config::write_json_file_with_contents;
use crate::error::AppError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn dsh_mcp_config_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// `<dsh home>/mcp.json`
pub fn get_dsh_mcp_config_path() -> PathBuf {
    crate::dsh_config::get_home().join("mcp.json")
}

fn read_mcp_config_from_path(path: &Path) -> Result<Value, AppError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // 文件不存在视为空配置（首次写入时创建）
            return Ok(json!({"servers": []}));
        }
        Err(err) => return Err(AppError::io(path, err)),
    };
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        AppError::Config(format!(
            "Failed to parse DSH mcp.json: {}: {e}",
            path.display()
        ))
    })?;

    // 根节点必须是对象：数组/标量属于用户损坏或异构文件，
    // 报错而不是重建根节点，避免覆盖用户自有配置。
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "DSH mcp.json 根节点必须是 JSON 对象: {}",
            path.display()
        )));
    }
    Ok(value)
}

fn write_mcp_config_to_path(path: &Path, config: &Value) -> Result<(), AppError> {
    // 确保父目录存在（dsh home 可能尚未创建）
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    write_json_file_with_contents(path, config)?;
    log::debug!("DSH mcp.json written to {path:?}");
    Ok(())
}

/// 取 servers 数组的可变引用；缺失则初始化为空数组，存在但非数组则报错。
/// 先结束键检查的借用再取 mut，避免 E0499 双重可变借用。
fn ensure_servers_array_mut<'a>(
    config: &'a mut Value,
    path: &Path,
) -> Result<&'a mut Vec<Value>, AppError> {
    let has_key = config
        .as_object()
        .map(|obj| obj.contains_key("servers"))
        .unwrap_or(false);
    if !has_key {
        config["servers"] = json!([]);
    }
    config
        .get_mut("servers")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            AppError::Config(format!(
                "DSH mcp.json 的 servers 字段必须是数组: {}",
                path.display()
            ))
        })
}

/// 读取 servers 数组，返回 `(name, entry)` 列表（保留文件顺序）
pub fn get_servers() -> Result<Vec<(String, Value)>, AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let config = read_mcp_config_from_path(&get_dsh_mcp_config_path())?;
    match config.get("servers") {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match name {
                    Some(name) => out.push((name, item.clone())),
                    None => {
                        return Err(AppError::Config(
                            "DSH mcp.json servers 数组中存在缺少 name 字段的条目".into(),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(AppError::Config(
            "DSH mcp.json 的 servers 字段必须是数组".into(),
        )),
    }
}

/// 按 name 写入/更新条目；同名替换、异名追加，未知条目与顶层字段原样保留
pub fn upsert_server(name: &str, entry: Value) -> Result<(), AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let path = get_dsh_mcp_config_path();
    let mut config = read_mcp_config_from_path(&path)?;
    let servers = ensure_servers_array_mut(&mut config, &path)?;
    if let Some(slot) = servers
        .iter_mut()
        .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
    {
        *slot = entry;
    } else {
        servers.push(entry);
    }
    write_mcp_config_to_path(&path, &config)
}

/// 按 name 移除条目（其他条目与顶层字段原样保留）
pub fn remove_server(name: &str) -> Result<(), AppError> {
    let _guard = dsh_mcp_config_lock().lock()?;
    let path = get_dsh_mcp_config_path();
    let mut config = read_mcp_config_from_path(&path)?;
    let servers = ensure_servers_array_mut(&mut config, &path)?;
    servers.retain(|item| item.get("name").and_then(Value::as_str) != Some(name));
    write_mcp_config_to_path(&path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use serde_json::json;

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
    fn mcp_path_defaults_to_home_dot_dsh() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        assert_eq!(
            path.file_name().map(std::ffi::OsStr::to_str),
            Some(Some("mcp.json"))
        );
        assert!(path.parent().is_some_and(|p| p.ends_with(".dsh")));
    }

    #[test]
    #[serial]
    fn get_servers_empty_when_file_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let servers = get_servers().expect("empty servers");
        assert!(servers.is_empty());
    }

    #[test]
    #[serial]
    fn upsert_server_roundtrip_and_preserves_unknown_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        // 预置一条 cc-switch 不认识的条目（模拟 dsh 侧手工配置）
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            r#"{"topLevelKept": true, "servers": [{"name": "user-own", "transport": "stdio", "command": "keep"}]}"#,
        )
        .expect("seed mcp.json");

        upsert_server(
            "echo",
            json!({"name": "echo", "transport": "stdio", "command": "npx"}),
        )
        .expect("upsert");

        let servers = get_servers().expect("read");
        let names: Vec<&str> = servers.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"user-own"), "unknown entry preserved");
        assert!(names.contains(&"echo"));
        let raw = std::fs::read_to_string(&path).expect("raw read");
        assert!(raw.contains("topLevelKept"), "top-level fields preserved");

        // 同名再写 = 更新而非追加
        upsert_server(
            "echo",
            json!({"name": "echo", "transport": "stdio", "command": "updated"}),
        )
        .expect("upsert again");
        let servers = get_servers().expect("read");
        assert_eq!(
            servers.iter().filter(|(n, _)| n == "echo").count(),
            1,
            "same-name upsert replaces, not appends"
        );
    }

    #[test]
    #[serial]
    fn remove_server_preserves_others() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());

        upsert_server(
            "keep",
            json!({"name": "keep", "transport": "stdio", "command": "k"}),
        )
        .expect("seed keep");
        upsert_server(
            "drop",
            json!({"name": "drop", "transport": "stdio", "command": "d"}),
        )
        .expect("seed drop");

        remove_server("drop").expect("remove");

        let servers = get_servers().expect("read");
        assert!(servers.iter().any(|(n, _)| n == "keep"));
        assert!(servers.iter().all(|(n, _)| n != "drop"));
    }

    #[test]
    #[serial]
    fn root_not_object_is_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, r#"[1, 2, 3]"#).expect("seed array root");

        assert!(get_servers().is_err(), "array root must be rejected");
        assert!(
            upsert_server("x", json!({"name": "x"})).is_err(),
            "must not overwrite user file with rebuilt root"
        );
    }

    #[test]
    #[serial]
    fn servers_not_array_is_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, r#"{"servers": {"echo": {}}}"#).expect("seed map servers");

        assert!(get_servers().is_err(), "object servers must be rejected");
    }

    #[test]
    #[serial]
    fn entry_missing_name_is_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestEnvGuard::set(temp.path());
        let path = get_dsh_mcp_config_path();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &path,
            r#"{"servers": [{"transport": "stdio", "command": "x"}]}"#,
        )
        .expect("seed nameless entry");

        assert!(get_servers().is_err(), "entry without name must be rejected");
    }
}
