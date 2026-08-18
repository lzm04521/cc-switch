//! 悬浮球相关命令：面板显隐、位置保存、分组数据、开关联动

#![allow(non_snake_case)]

use tauri::AppHandle;
use tauri::Manager;

use crate::floating_ball::BallSection;
use crate::store::AppState;

/// 切换面板显隐，返回 "opened" / "closed"
#[tauri::command]
pub fn toggle_ball_panel(app: AppHandle) -> Result<&'static str, String> {
    crate::floating_ball::toggle_panel(&app)
}

/// 直接隐藏面板（Esc / 切换成功后前端调用）
#[tauri::command]
pub fn hide_ball_panel(app: AppHandle) -> Result<bool, String> {
    crate::floating_ball::hide_panel(&app);
    Ok(true)
}

/// 面板失焦回调（前端 onFocusChanged(false) 触发）
#[tauri::command]
pub fn on_ball_panel_blur(app: AppHandle) -> Result<bool, String> {
    crate::floating_ball::on_panel_blur(&app);
    Ok(true)
}

/// 拖拽结束后持久化悬浮球位置
#[tauri::command]
pub fn save_ball_position(app: AppHandle) -> Result<bool, String> {
    crate::floating_ball::save_ball_position(&app)
}

/// 开始悬浮球拖动（Windows 分层窗口由后端原生循环自驱动，全程球体可见）
#[tauri::command]
pub fn start_ball_drag(app: AppHandle) -> Result<(), String> {
    crate::floating_ball::start_ball_drag(&app)
}

/// 前端 hover 上报：进入贴边露条 → 展开；离开 → 延迟收回（自由态忽略）
#[tauri::command]
pub fn on_ball_hover(app: AppHandle, entered: bool) -> Result<bool, String> {
    crate::floating_ball::ball_hover(&app, entered)
}

/// 设置页 / 托盘开关调用：更新设置并立即同步窗口可见性
#[tauri::command]
pub fn set_floating_ball_enabled(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let mut settings = crate::settings::get_settings();
    settings.floating_ball.enabled = enabled;
    crate::settings::update_settings(settings).map_err(|e| e.to_string())?;
    crate::floating_ball::ensure_ball_window(&app);
    Ok(true)
}

/// 一次返回所有分组的 provider 列表 + 每 app 当前 provider
#[tauri::command]
pub async fn get_floating_ball_sections(app: AppHandle) -> Result<Vec<BallSection>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "应用状态不可用".to_string())?;
        crate::floating_ball::build_sections(state.inner()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("构建悬浮球分组任务失败: {e}"))?
}

/// 打开主窗口（面板 footer 调用；复用托盘逻辑，含退出轻量模式）
#[tauri::command]
pub fn show_main_window(app: AppHandle) -> Result<bool, String> {
    crate::tray::open_main_window(&app);
    Ok(true)
}
