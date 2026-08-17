//! 悬浮球（快速切换 Provider 的置顶悬浮窗）窗口管理
//!
//! ball 窗口：56px 圆形置顶小窗，显示固定 CC Switch 图标，可拖动、位置持久化。
//! panel 窗口：点击球后在其旁边弹出的 provider 分组列表。
//! 两个窗口均在 tauri.conf.json 预配置（visible: false），启动时由 Tauri 自动创建。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, PhysicalPosition};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

pub const BALL_LABEL: &str = "ball";
pub const PANEL_LABEL: &str = "panel";
/// 面板逻辑尺寸（与 tauri.conf.json 中 panel 窗口配置一致）
pub const PANEL_WIDTH: f64 = 300.0;
pub const PANEL_HEIGHT: f64 = 480.0;
/// 面板与球之间的间距（逻辑像素）
pub const PANEL_GAP: f64 = 8.0;

/// 面板当前是否可见（状态机单一事实源，所有显隐操作经 toggle/hide 修改）
static PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// 逻辑坐标矩形
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 计算面板位置（坐标单位由调用方决定，逻辑/物理需保持一致）：
/// 优先出现在球右侧，放不下翻到左侧，垂直以球中心对齐，
/// 最终钳制在显示器 work area 内。
/// work area 比面板还小时直接贴边（x/y 取 work 左上角）。
pub fn compute_panel_position(
    ball: &Rect,
    work: &Rect,
    panel_width: f64,
    panel_height: f64,
    gap: f64,
) -> (f64, f64) {
    // 水平：优先右侧，放不下翻左侧
    let mut x = ball.x + ball.width + gap;
    if x + panel_width > work.x + work.width {
        x = ball.x - gap - panel_width;
    }
    // 钳制：面板比 work area 宽或翻转后仍越界时贴边
    x = x.min(work.x + work.width - panel_width).max(work.x);

    // 垂直：以球中心对齐，钳制在 work area 内
    let mut y = ball.y + ball.height / 2.0 - panel_height / 2.0;
    y = y.min(work.y + work.height - panel_height).max(work.y);

    (x, y)
}

/// 按设置同步悬浮球窗口可见性：启用则显示球，关闭则收起面板并隐藏球。
/// 两个窗口由 tauri.conf 预配置、启动时自动创建，这里只负责 show/hide。
pub fn ensure_ball_window(app: &AppHandle) {
    let enabled = crate::settings::get_settings().floating_ball.enabled;
    let Some(ball) = app.get_webview_window(BALL_LABEL) else {
        log::error!("悬浮球窗口未找到（tauri.conf.json 缺少 ball 窗口配置）");
        return;
    };
    if enabled {
        if let Err(e) = ball.show() {
            log::error!("显示悬浮球窗口失败: {e}");
        }
    } else {
        hide_panel(app);
        if let Err(e) = ball.hide() {
            log::error!("隐藏悬浮球窗口失败: {e}");
        }
    }
}

/// 隐藏面板（幂等；blur / 切换成功 / 设置关闭共用）
pub fn hide_panel(app: &AppHandle) {
    if PANEL_VISIBLE.swap(false, Ordering::AcqRel) {
        if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
            if let Err(e) = panel.hide() {
                log::error!("隐藏悬浮球面板失败: {e}");
            }
        }
    }
}

/// 切换面板可见性。返回 "opened" / "closed"，前端据此判断是否需要重开。
/// 使用原子量 swap 序列化所有显隐操作，杜绝并发竞态（点球关面板防闪烁）。
pub fn toggle_panel(app: &AppHandle) -> Result<&'static str, String> {
    // 已可见 → 关闭
    if PANEL_VISIBLE.swap(true, Ordering::AcqRel) {
        hide_panel(app);
        return Ok("closed");
    }

    // 打开过程中任一可失败步骤（取窗口/显示器/定位/show）失败都会复位 flag，
    // 避免卡在 true 导致下次点击误走上面的关闭分支（面板其实并未显示）。
    if let Err(e) = show_panel(app) {
        PANEL_VISIBLE.store(false, Ordering::Release);
        return Err(e);
    }
    Ok("opened")
}

/// 打开面板：定位 + 显示。可失败步骤集中在此，失败时由 toggle_panel 复位状态。
fn show_panel(app: &AppHandle) -> Result<(), String> {
    let panel = app
        .get_webview_window(PANEL_LABEL)
        .ok_or("悬浮球面板窗口未初始化")?;
    let ball = app
        .get_webview_window(BALL_LABEL)
        .ok_or("悬浮球窗口未初始化")?;

    // 定位：基于球窗口位置，钳制在当前显示器 work area 内（多显示器安全）。
    // 全程用物理像素（全局虚拟桌面坐标）计算，再用 PhysicalPosition 设置——
    // 若改用 LogicalPosition，set_position 会按 panel 窗口所在显示器的缩放
    // 换算回物理坐标，在混合 DPI（如 16 寸 150% + 外接 14 寸 100%）下
    // 面板会被放到屏幕外，表现为"拖到扩展屏后点不出菜单"。
    let monitor = ball
        .current_monitor()
        .map_err(|e| format!("获取显示器失败: {e}"))?
        .ok_or("无法确定悬浮球所在显示器")?;
    let scale = monitor.scale_factor();
    let ball_pos = ball
        .outer_position()
        .map_err(|e| format!("获取悬浮球位置失败: {e}"))?;
    let ball_size = ball
        .inner_size()
        .map_err(|e| format!("获取悬浮球尺寸失败: {e}"))?;
    // work_area 返回物理像素矩形（&PhysicalRect），直接使用无需换算
    let work = monitor.work_area();

    let (px, py) = compute_panel_position(
        &Rect {
            x: ball_pos.x as f64,
            y: ball_pos.y as f64,
            width: ball_size.width as f64,
            height: ball_size.height as f64,
        },
        &Rect {
            x: work.position.x as f64,
            y: work.position.y as f64,
            width: work.size.width as f64,
            height: work.size.height as f64,
        },
        PANEL_WIDTH * scale,
        PANEL_HEIGHT * scale,
        PANEL_GAP * scale,
    );

    panel
        .set_position(PhysicalPosition::new(px.round() as i32, py.round() as i32))
        .map_err(|e| format!("设置面板位置失败: {e}"))?;
    panel.show().map_err(|e| format!("显示面板失败: {e}"))?;
    let _ = panel.set_focus();
    Ok(())
}

/// 面板失焦回调（前端 onFocusChanged(false) 触发）：
/// 若新焦点落在球窗上，关闭动作交给球的 toggle（点球关面板），避免重复处理。
pub fn on_panel_blur(app: &AppHandle) {
    // get_focused_window 需 unstable feature，改用 webview_windows 遍历找焦点窗口
    let focus_on_ball = app
        .webview_windows()
        .values()
        .find(|w| w.is_focused().unwrap_or(false))
        .map(|w| w.label() == BALL_LABEL)
        .unwrap_or(false);
    if !focus_on_ball {
        hide_panel(app);
    }
}

/// 拖拽结束后持久化窗口位置（window-state 插件落盘，重启自动恢复）
pub fn save_ball_position(app: &AppHandle) -> Result<bool, String> {
    app.save_window_state(StateFlags::POSITION)
        .map(|_| true)
        .map_err(|e| format!("保存悬浮球位置失败: {e}"))
}

/// 开始悬浮球拖动。
///
/// Windows：ball 是 transparent 分层窗口，系统 move loop（start_dragging）只画
/// "拖动外框"、窗口内容留在原地；而前端 pointermove + setPosition 逐帧 IPC
/// 驱动又受 IPC 调度抖动影响会一卡一卡。这里改为后台线程轮询光标 +
/// SetWindowPos 自驱动（约 500Hz），球体全程显示、零 IPC 延迟，手感对齐
/// PixPin。拖动结束（左键释放）后落盘位置。
///
/// 其它平台：分层窗口无此问题，直接使用系统 move loop。
#[cfg(target_os = "windows")]
pub fn start_ball_drag(app: &AppHandle) -> Result<(), String> {
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER,
    };

    let window = app
        .get_webview_window(BALL_LABEL)
        .ok_or("悬浮球窗口未初始化")?;
    // tauri 的 hwnd() 返回 windows crate 的 HWND，其 .0 为 *mut c_void，
    // 与 windows-sys 的 HWND 底层一致，直接取出即可
    let hwnd: HWND = window
        .hwnd()
        .map_err(|e| format!("获取悬浮球窗口句柄失败: {e}"))?
        .0;

    // 进入拖动时光标相对窗口左上角的偏移（物理像素），拖动中保持不变
    let mut cursor = POINT { x: 0, y: 0 };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetCursorPos(&mut cursor);
        GetWindowRect(hwnd, &mut rect);
    }
    let offset_x = cursor.x - rect.left;
    let offset_y = cursor.y - rect.top;

    let app = app.clone();
    // HWND（*mut c_void）不是 Send，转成 isize 传入线程后再还原
    let hwnd_value = hwnd as isize;
    std::thread::spawn(move || {
        let hwnd = hwnd_value as HWND;
        loop {
            // 约 500Hz 轮询：每 tick 同步光标位置并移动窗口，均匀无抖动
            std::thread::sleep(Duration::from_millis(2));
            // 左键释放即结束拖动
            if (unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16) & 0x8000 == 0 {
                break;
            }
            let mut cur = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut cur) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    cur.x - offset_x,
                    cur.y - offset_y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    });
    // 拖动结束，落盘位置（window-state 插件对 Moved 事件的防抖保存作兜底）
    let _ = app.save_window_state(StateFlags::POSITION);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn start_ball_drag(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(BALL_LABEL)
        .ok_or("悬浮球窗口未初始化")?;
    window
        .start_dragging()
        .map_err(|e| format!("启动悬浮球拖动失败: {e}"))
}

/// 分组内单个 provider 的展示信息（图标由前端本地渲染）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BallProviderInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_color: Option<String>,
}

/// 面板的一个 app 分组
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BallSection {
    pub app_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_provider_id: Option<String>,
    pub providers: Vec<BallProviderInfo>,
}

/// 构建面板分组数据：按 AppType 顺序遍历，过滤不可见应用。
/// additive 模式（OpenCode/OpenClaw/Hermes/Pi）无单一“当前供应商”，
/// current_provider_id 为 None（不渲染勾选），点击行为与主界面一致（switch_provider）。
pub fn build_sections(
    app_state: &crate::store::AppState,
) -> Result<Vec<BallSection>, crate::error::AppError> {
    let app_settings = crate::settings::get_settings();
    let visible_apps = app_settings.visible_apps.unwrap_or_default();
    let mut sections = Vec::new();

    for app_type in crate::app_config::AppType::all() {
        // zcode 的 provider 由 ZCode 应用内自管，cc-switch 不写入/不切换，
        // 悬浮球（快速切换入口）不展示该分组
        if matches!(app_type, crate::app_config::AppType::Zcode) {
            continue;
        }
        if !visible_apps.is_visible(&app_type) {
            continue;
        }
        let providers = app_state.db.get_all_providers(app_type.as_str())?;
        let current_provider_id = if app_type.is_additive_mode() {
            None
        } else {
            crate::settings::get_effective_current_provider(&app_state.db, &app_type)?
        };
        let sorted = crate::tray::sort_providers(&providers);
        let provider_infos = sorted
            .into_iter()
            .map(|(id, p)| BallProviderInfo {
                id: id.clone(),
                name: p.name.clone(),
                icon: p.icon.clone(),
                icon_color: p.icon_color.clone(),
            })
            .collect();
        sections.push(BallSection {
            app_type: app_type.as_str().to_string(),
            current_provider_id,
            providers: provider_infos,
        });
    }
    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn panel_places_right_of_ball_when_space_allows() {
        let ball = rect(400.0, 300.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let (x, y) = compute_panel_position(&ball, &work, 300.0, 480.0, PANEL_GAP);
        assert_eq!(x, 400.0 + 56.0 + PANEL_GAP);
        // 面板垂直居中于球：球中心 y = 300 + 28 = 328，面板中心 = 328 → y = 328 - 240
        assert!((y - 88.0).abs() < 1e-6);
    }

    #[test]
    fn panel_flips_left_near_right_edge() {
        let ball = rect(1850.0, 300.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let (x, _) = compute_panel_position(&ball, &work, 300.0, 480.0, PANEL_GAP);
        assert_eq!(x, 1850.0 - PANEL_GAP - 300.0);
    }

    #[test]
    fn panel_clamps_into_small_work_area() {
        // 副屏矮工作区：面板高度放不下 → 钳制贴边
        let ball = rect(100.0, 2.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 800.0, 300.0);
        let (x, y) = compute_panel_position(&ball, &work, 300.0, 480.0, PANEL_GAP);
        assert!(x >= 0.0 && x + 300.0 <= 800.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn panel_never_exceeds_work_area_when_panel_bigger() {
        let ball = rect(0.0, 0.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 200.0, 200.0);
        let (x, y) = compute_panel_position(&ball, &work, 300.0, 480.0, PANEL_GAP);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn panel_clamps_into_secondary_monitor_with_nonzero_origin() {
        // 模拟副屏在主屏右侧的物理坐标：work 原点非 0（混合 DPI 下用物理像素计算，
        // 球/面板尺寸按 1.5x 缩放换算）。面板必须钳制在副屏 work area 内，
        // 不能被 clamp 到主屏（x=0）一侧。
        let ball = rect(3200.0, 400.0, 84.0, 84.0);
        let work = rect(2880.0, 0.0, 1920.0, 1040.0);
        let (x, y) = compute_panel_position(&ball, &work, 450.0, 720.0, 12.0);
        assert!(x >= 2880.0 && x + 450.0 <= 2880.0 + 1920.0);
        assert!(y >= 0.0 && y + 720.0 <= 1040.0);
    }
}
