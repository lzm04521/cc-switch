//! The DeepSeek account login shared by DSH Web and Desktop.

/// The DeepSeek account (OAuth) login shared by DSH Web and Desktop, with profile and balance.
#[tauri::command]
pub async fn dsh_account_status() -> Result<crate::services::dsh_account::DshAccountStatus, String>
{
    crate::services::dsh_account::status().await
}

/// Sign out of the DeepSeek account for both DSH Web and Desktop.
#[tauri::command]
pub async fn dsh_account_sign_out() -> Result<(), String> {
    crate::services::dsh_account::sign_out().await
}

/// Start a DeepSeek account browser sign-in; the UI opens `authUrl` and polls.
#[tauri::command]
pub async fn dsh_account_login_start() -> Result<crate::services::dsh_account::DshLoginStart, String>
{
    crate::services::dsh_account::start_login().await
}

/// `false` while waiting for the browser, `true` once the shared login is stored.
#[tauri::command(rename_all = "camelCase")]
pub fn dsh_account_login_poll(flow_id: String) -> Result<bool, String> {
    crate::services::dsh_account::poll_login(&flow_id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn dsh_account_login_cancel(flow_id: String) {
    crate::services::dsh_account::cancel_login(&flow_id)
}

// ── fork 定制：DSH home 目录管理（设置页目录覆盖 UI 支撑） ──────────────

/// Current DSH home honoring cc-switch's directory override.
#[tauri::command]
pub fn dsh_get_home() -> Result<String, String> {
    Ok(crate::dsh_config::data_dir().to_string_lossy().into_owned())
}

/// The DSH home cc-switch falls back to (no directory override applied).
#[tauri::command]
pub fn dsh_get_default_home() -> Result<String, String> {
    Ok(crate::dsh_config::get_default_home()
        .to_string_lossy()
        .into_owned())
}

/// Open the DSH home directory in the system file manager.
#[tauri::command]
pub fn dsh_open_home(handle: tauri::AppHandle) -> Result<(), String> {
    let home = crate::dsh_config::data_dir();
    crate::dsh_config::ensure_secure_home(&home)?;
    use tauri_plugin_opener::OpenerExt;
    handle
        .opener()
        .open_path(home.to_string_lossy().into_owned(), None::<String>)
        .map_err(|source| source.to_string())
}
