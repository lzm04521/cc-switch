//! Tauri commands for DSH (DeepSeek Harness) home directory management.

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn dsh_get_home() -> Result<String, String> {
    Ok(crate::dsh_config::get_home().to_string_lossy().to_string())
}

#[tauri::command]
pub fn dsh_get_default_home() -> Result<String, String> {
    Ok(crate::dsh_config::get_default_home()
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub fn dsh_open_home(handle: AppHandle) -> Result<(), String> {
    let home = crate::dsh_config::get_home();
    crate::dsh_config::ensure_secure_home(&home)?;
    handle
        .opener()
        .open_path(home.to_string_lossy().to_string(), None::<String>)
        .map_err(|source| source.to_string())
}
