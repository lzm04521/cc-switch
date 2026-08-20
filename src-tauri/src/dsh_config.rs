//! DSH (DeepSeek Harness) home directory resolution.
//!
//! cc-switch 同时维护 `<dsh home>/profiles/{web,desktop}/cordis.patch.yml`
//! （MCP，DSH 客户端部分走 web、部分走 desktop）与 skills 部署目录，不解析
//! settings.yaml / .credentials.yaml（provider 由 DSH 应用内自管）。
//! home 三级解析：设置覆盖（dsh_config_dir）→ `DSH_HOME` 环境变量 → `~/.dsh`。

use std::fs;
use std::path::{Path, PathBuf};

pub fn get_home() -> PathBuf {
    if let Some(override_dir) = crate::settings::get_dsh_override_dir() {
        return override_dir;
    }
    get_default_home()
}

/// Resolve the DSH home without consulting cc-switch's directory override.
///
/// The settings page uses this when resetting an override, so an environment
/// supplied `DSH_HOME` is restored instead of being replaced by `~/.dsh`.
pub fn get_default_home() -> PathBuf {
    if let Some(raw) = std::env::var_os("DSH_HOME") {
        let trimmed = raw.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            return expand_home(&trimmed);
        }
    }
    crate::config::get_home_dir().join(".dsh")
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return crate::config::get_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return crate::config::get_home_dir().join(rest);
    }
    PathBuf::from(raw)
}

/// Create the DSH home directory when missing; tighten permissions on Unix for
/// newly created homes (credentials may live alongside profile configs).
pub(crate) fn ensure_secure_home(home: &Path) -> Result<(), String> {
    // Must be captured before create_dir_all; only newly created homes get
    // their permissions tightened on Unix. The variable is cfg'd so Windows
    // builds do not warn about an unused binding.
    #[cfg(unix)]
    let existed = home.exists();
    fs::create_dir_all(home).map_err(|source| {
        io_error("create-home-failed", "Failed to create DSH home", source)
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !existed {
            fs::set_permissions(home, fs::Permissions::from_mode(0o700)).map_err(|source| {
                io_error(
                    "permissions-failed",
                    "Failed to secure DSH home permissions",
                    source,
                )
            })?;
        }
    }
    Ok(())
}

fn error(code: &str, message: impl Into<String>) -> String {
    format!("{}: {}", code, message.into())
}

fn io_error(code: &str, context: &str, source: std::io::Error) -> String {
    error(code, format!("{context}: {source}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    // 两个测试读取进程级 env（DSH_HOME/CC_SWITCH_TEST_HOME），与同模块族
    // 带 env guard 的 #[serial] 测试互斥，避免并行读到被改的环境
    use serial_test::serial;

    #[test]
    #[serial]
    fn default_home_is_dot_dsh_under_home_dir() {
        assert_eq!(
            get_default_home(),
            crate::config::get_home_dir().join(".dsh")
        );
    }

    #[test]
    #[serial]
    fn expand_home_handles_tilde_forms() {
        let home = crate::config::get_home_dir();
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/.custom"), home.join(".custom"));
        assert_eq!(
            expand_home("~\\.custom"),
            home.join(".custom"),
            "Windows 风格分隔符同样展开"
        );
        assert_eq!(expand_home("/abs/path"), PathBuf::from("/abs/path"));
    }
}
