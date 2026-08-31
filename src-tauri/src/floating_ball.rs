//! 悬浮球（快速切换 Provider 的置顶悬浮窗）窗口管理
//!
//! ball 窗口：56px 圆形置顶小窗，显示固定 CC Switch 图标，可拖动、位置持久化，
//! 拖近屏幕左右边缘松手自动贴边收起（只露一小条），hover 露条滑出展开。
//! panel 窗口：点击球后在其旁边弹出的 provider 分组列表。
//! 两个窗口均在 tauri.conf.json 预配置（visible: false），启动时由 Tauri 自动创建。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

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

// ===== 贴边隐藏（dock）=====
//
// 交互：拖近左右边缘松手 → 吸附贴边并滑出收起（只露一小条）；
// hover 露条 → 滑出展开；鼠标移开 → 延迟收回；拖离边缘 → 恢复自由摆放。
// 重启时由已保存的真实窗口位置反推 dock 状态（不新增设置字段）。

/// 吸附判定阈值（物理像素）：松手时球与所在屏 work area 左/右边缘距离小于该值即贴边
const SNAP_THRESHOLD_PX: f64 = 32.0;
/// 收起时露出量（逻辑像素，按显示器 DPI 换算为物理像素）
const COLLAPSED_VISIBLE_LOGICAL_PX: f64 = 10.0;
/// 贴边滑动动画总时长与帧间隔
const DOCK_ANIMATE_MS: u64 = 150;
const DOCK_ANIMATE_FRAME_MS: u64 = 8;
/// mouseleave / 面板关闭后延迟收回的等待时长
const RECOLLAPSE_DELAY_MS: u64 = 400;
/// 启动恢复判定容差（物理像素，按 DPI 缩放）：位置与收起/展开位偏差在该值内视为对应状态
const RESTORE_TOLERANCE_PX: f64 = 4.0;
/// 启动首显兜底定时：主窗口页面加载完成事件未触发时，延迟该时长后显示悬浮球
pub const STARTUP_FALLBACK_MS: u64 = 3000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// 球当前 dock 状态：None = 自由摆放
static DOCK: OnceLock<Mutex<Option<DockSide>>> = OnceLock::new();
/// dock 态下球是否处于收起位
static DOCK_COLLAPSED: AtomicBool = AtomicBool::new(false);
/// 后端拖动循环进行中（hover 展开在拖动期间被忽略）
static BALL_DRAGGING: AtomicBool = AtomicBool::new(false);
/// 贴边滑动动画进行中（互斥：拖动开始 / 新动画 / 收回判定）
static BALL_ANIMATING: AtomicBool = AtomicBool::new(false);
/// 动画代数：新动画 / 拖动开始时递增，旧动画线程发现代数变化即自杀，
/// 避免拖动与动画两个 SetWindowPos 循环同时驱动窗口产生抖动
static DOCK_ANIM_GEN: AtomicU64 = AtomicU64::new(0);
/// 启动首显闸门：页面加载事件与兜底定时竞争触发，仅第一次生效
static BALL_STARTUP_SHOWN: AtomicBool = AtomicBool::new(false);

fn dock_state() -> &'static Mutex<Option<DockSide>> {
    DOCK.get_or_init(|| Mutex::new(None))
}

fn set_dock(side: Option<DockSide>, collapsed: bool) {
    *dock_state().lock().unwrap_or_else(|e| e.into_inner()) = side;
    DOCK_COLLAPSED.store(collapsed, Ordering::Release);
}

/// 判定任务栏所在侧：比较显示器全屏矩形与 work area 的差值。
/// 任务栏隐藏 / 副屏无任务栏时返回 None（四边均可收缩）。
fn taskbar_side(monitor: &Rect, work: &Rect) -> Option<DockSide> {
    if work.y > monitor.y {
        Some(DockSide::Top)
    } else if work.x > monitor.x {
        Some(DockSide::Left)
    } else if work.x + work.width < monitor.x + monitor.width {
        Some(DockSide::Right)
    } else if work.y + work.height < monitor.y + monitor.height {
        Some(DockSide::Bottom)
    } else {
        None
    }
}

/// 判定松手位置是否应贴边：取与 work area 四边更近的一侧，距离须小于阈值，
/// 且跳过任务栏所在侧（收缩窗口会盖在任务栏上，视觉混乱）。
fn should_snap(ball: &Rect, work: &Rect, monitor: &Rect, threshold: f64) -> Option<DockSide> {
    let blocked = taskbar_side(monitor, work);
    let dists = [
        ((ball.x - work.x).abs(), DockSide::Left),
        (
            (work.x + work.width - ball.x - ball.width).abs(),
            DockSide::Right,
        ),
        ((ball.y - work.y).abs(), DockSide::Top),
        (
            (work.y + work.height - ball.y - ball.height).abs(),
            DockSide::Bottom,
        ),
    ];
    let mut best: Option<(f64, DockSide)> = None;
    for (d, side) in dists {
        if Some(side) == blocked || d >= threshold {
            continue;
        }
        if best.map_or(true, |(bd, _)| d < bd) {
            best = Some((d, side));
        }
    }
    best.map(|(_, side)| side)
}

/// 贴边展开位（完全可见，贴 work area 边缘；非收缩轴向保持球的当前坐标）
fn dock_expanded_pos(side: DockSide, work: &Rect, ball: &Rect) -> (f64, f64) {
    match side {
        DockSide::Left => (work.x, ball.y),
        DockSide::Right => (work.x + work.width - ball.width, ball.y),
        DockSide::Top => (ball.x, work.y),
        DockSide::Bottom => (ball.x, work.y + work.height - ball.height),
    }
}

/// 贴边收起位（滑出屏外，只露 visible_px）
fn dock_collapsed_pos(side: DockSide, work: &Rect, ball: &Rect, visible_px: f64) -> (f64, f64) {
    match side {
        DockSide::Left => (work.x - ball.width + visible_px, ball.y),
        DockSide::Right => (work.x + work.width - visible_px, ball.y),
        DockSide::Top => (ball.x, work.y - ball.height + visible_px),
        DockSide::Bottom => (ball.x, work.y + work.height - visible_px),
    }
}

#[cfg(target_os = "windows")]
mod win32_dock {
    use std::sync::atomic::Ordering;

    use tauri::{AppHandle, Manager};
    use tauri_plugin_window_state::{AppHandleExt, StateFlags};
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowRect, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER,
    };

    use super::{Rect, BALL_ANIMATING, DOCK_ANIM_GEN};

    pub(super) fn ball_hwnd(app: &AppHandle) -> Result<HWND, String> {
        let window = app
            .get_webview_window(super::BALL_LABEL)
            .ok_or("悬浮球窗口未初始化")?;
        window
            .hwnd()
            .map(|h| h.0)
            .map_err(|e| format!("获取悬浮球窗口句柄失败: {e}"))
    }

    /// 取球所在显示器的全屏矩形与 work area（物理像素，全局虚拟桌面坐标）。
    /// monitor 矩形用于判定任务栏所在侧（与 work area 的差值）。
    pub(super) fn ball_monitor_rects(hwnd: HWND) -> Option<(Rect, Rect)> {
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return None;
        }
        let full = Rect {
            x: info.rcMonitor.left as f64,
            y: info.rcMonitor.top as f64,
            width: (info.rcMonitor.right - info.rcMonitor.left) as f64,
            height: (info.rcMonitor.bottom - info.rcMonitor.top) as f64,
        };
        let work = Rect {
            x: info.rcWork.left as f64,
            y: info.rcWork.top as f64,
            width: (info.rcWork.right - info.rcWork.left) as f64,
            height: (info.rcWork.bottom - info.rcWork.top) as f64,
        };
        Some((full, work))
    }

    /// 球窗口的显示缩放系数（按窗口 DPI 换算，供逻辑→物理像素转换）
    pub(super) fn ball_scale(hwnd: HWND) -> f64 {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi == 0 {
            1.0
        } else {
            dpi as f64 / 96.0
        }
    }

    pub(super) fn window_rect(hwnd: HWND) -> Rect {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut rect) };
        Rect {
            x: rect.left as f64,
            y: rect.top as f64,
            width: (rect.right - rect.left) as f64,
            height: (rect.bottom - rect.top) as f64,
        }
    }

    /// 光标当前是否落在球窗口矩形内（用于收回前的"鼠标还在球上"校验）
    pub(super) fn cursor_over_ball(hwnd: HWND) -> bool {
        let mut cur = POINT { x: 0, y: 0 };
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe {
            GetCursorPos(&mut cur);
            GetWindowRect(hwnd, &mut rect);
        }
        cur.x >= rect.left && cur.x < rect.right && cur.y >= rect.top && cur.y < rect.bottom
    }

    fn move_ball_to(hwnd: HWND, x: i32, y: i32) {
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    fn persist_position(app: &AppHandle) {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = handle.save_window_state(StateFlags::POSITION);
        });
    }

    /// 将球移动到 (target_x, target_y)（物理像素），线性动画（双轴插值）。
    /// 结束后落盘。启动时递增动画代数：在途旧动画被新动画/拖动取代后自动退出。
    pub(super) fn animate_ball_to(app: &AppHandle, hwnd: HWND, target_x: i32, target_y: i32) {
        let rect = window_rect(hwnd);
        let (start_x, start_y) = (rect.x as i32, rect.y as i32);
        if (target_x - start_x).abs() <= 1 && (target_y - start_y).abs() <= 1 {
            persist_position(app);
            return;
        }
        let gen = DOCK_ANIM_GEN.fetch_add(1, Ordering::AcqRel) + 1;
        BALL_ANIMATING.store(true, Ordering::Release);
        // HWND（*mut c_void）不是 Send，转成 isize 传入线程后再还原
        let hwnd_value = hwnd as isize;
        let app = app.clone();
        std::thread::spawn(move || {
            let hwnd = hwnd_value as HWND;
            let total_x = (target_x - start_x) as f64;
            let total_y = (target_y - start_y) as f64;
            let frames = (super::DOCK_ANIMATE_MS / super::DOCK_ANIMATE_FRAME_MS).max(1);
            for i in 1..=frames {
                std::thread::sleep(std::time::Duration::from_millis(
                    super::DOCK_ANIMATE_FRAME_MS,
                ));
                if DOCK_ANIM_GEN.load(Ordering::Acquire) != gen {
                    break;
                }
                let t = i as f64 / frames as f64;
                let x = start_x + (total_x * t).round() as i32;
                let y = start_y + (total_y * t).round() as i32;
                move_ball_to(hwnd, x, y);
            }
            if DOCK_ANIM_GEN.load(Ordering::Acquire) == gen {
                // 收尾精确落位（帧取整可能差 1px）
                move_ball_to(hwnd, target_x, target_y);
                BALL_ANIMATING.store(false, Ordering::Release);
                persist_position(&app);
                // 动画期间 hover 事件被 BALL_ANIMATING 拦截，鼠标可能已经停在
                // 球/露条上但 mouseenter 不会再触发；按当前光标位置补一次判定
                reconcile_after_dock_animation(&app, hwnd);
            }
        });
    }

    /// dock 动画结束后的状态对账：收起完成且鼠标在球上 → 立即展开；
    /// 展开完成且鼠标已移开 → 安排延迟收回（内部有完整校验，误触发无害）。
    fn reconcile_after_dock_animation(app: &AppHandle, hwnd: HWND) {
        let collapsed = super::DOCK_COLLAPSED.load(Ordering::Acquire);
        let over = cursor_over_ball(hwnd);
        if collapsed && over {
            let _ = super::ball_hover(app, true);
        } else if !collapsed && !over {
            schedule_recollapse(app);
        }
    }

    /// 延迟收回：等待 RECOLLAPSE_DELAY_MS 后复核状态（仍贴边展开、非拖动/动画、
    /// 面板已关、光标不在球上）才执行收起动画。点球关面板时鼠标仍在球上，
    /// 复核失败自然放弃；面板仍开着时不收（避免面板悬空）。
    pub(super) fn schedule_recollapse(app: &AppHandle) {
        use super::{
            BALL_ANIMATING, BALL_DRAGGING, COLLAPSED_VISIBLE_LOGICAL_PX, DOCK_COLLAPSED,
            PANEL_VISIBLE,
        };

        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(super::RECOLLAPSE_DELAY_MS));
            let docked = *super::dock_state()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Some(side) = docked else { return };
            if DOCK_COLLAPSED.load(Ordering::Acquire)
                || BALL_DRAGGING.load(Ordering::Acquire)
                || BALL_ANIMATING.load(Ordering::Acquire)
                || PANEL_VISIBLE.load(Ordering::Acquire)
            {
                return;
            }
            let Ok(hwnd) = ball_hwnd(&app) else { return };
            if cursor_over_ball(hwnd) {
                return;
            }
            let Some((_full, work)) = ball_monitor_rects(hwnd) else { return };
            let rect = window_rect(hwnd);
            let visible = COLLAPSED_VISIBLE_LOGICAL_PX * ball_scale(hwnd);
            let (tx, ty) = super::dock_collapsed_pos(side, &work, &rect, visible);
            DOCK_COLLAPSED.store(true, Ordering::Release);
            animate_ball_to(&app, hwnd, tx.round() as i32, ty.round() as i32);
        });
    }

    /// 拖动结束后处理贴边：吸附判定 → 收起动画 / 清除 dock；最终统一落盘。
    pub(super) fn finish_ball_drag(app: &AppHandle, hwnd: HWND) {
        let ball = window_rect(hwnd);
        let rects = ball_monitor_rects(hwnd);
        let side = rects.as_ref().and_then(|(full, work)| {
            super::should_snap(&ball, work, full, super::SNAP_THRESHOLD_PX)
        });
        match (side, rects) {
            (Some(side), Some((_full, work))) => {
                super::set_dock(Some(side), true);
                // 收起时同步收起面板（球即将滑出，面板不应悬空）
                let app_hide = app.clone();
                let _ = app.run_on_main_thread(move || super::hide_panel(&app_hide));
                let visible = super::COLLAPSED_VISIBLE_LOGICAL_PX * ball_scale(hwnd);
                let (tx, ty) = super::dock_collapsed_pos(side, &work, &ball, visible);
                animate_ball_to(app, hwnd, tx.round() as i32, ty.round() as i32);
            }
            _ => {
                super::set_dock(None, false);
                persist_position(app);
            }
        }
    }
}

/// 前端 hover 上报入口：进入露出条 → 展开；离开 → 延迟收回。
/// 返回值表示球当前是否处于 dock 态（自由态恒 false，前端不依赖返回值）。
#[cfg(target_os = "windows")]
pub fn ball_hover(app: &AppHandle, entered: bool) -> Result<bool, String> {
    let docked = *dock_state().lock().unwrap_or_else(|e| e.into_inner());
    let Some(side) = docked else { return Ok(false) };
    if entered {
        if DOCK_COLLAPSED.load(Ordering::Acquire)
            && !BALL_ANIMATING.load(Ordering::Acquire)
            && !BALL_DRAGGING.load(Ordering::Acquire)
        {
            let hwnd = win32_dock::ball_hwnd(app)?;
            let Some((_full, work)) = win32_dock::ball_monitor_rects(hwnd) else {
                return Ok(true);
            };
            let rect = win32_dock::window_rect(hwnd);
            let (tx, ty) = dock_expanded_pos(side, &work, &rect);
            DOCK_COLLAPSED.store(false, Ordering::Release);
            win32_dock::animate_ball_to(app, hwnd, tx.round() as i32, ty.round() as i32);
        }
    } else if !DOCK_COLLAPSED.load(Ordering::Acquire) {
        win32_dock::schedule_recollapse(app);
    }
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
pub fn ball_hover(_app: &AppHandle, _entered: bool) -> Result<bool, String> {
    Ok(false)
}

/// 启动 / 启用悬浮球时按已保存的窗口位置恢复 dock 状态（不动画，直接对号入座）。
/// window-state 插件保存的是收起/展开时的真实物理位置，据此反推。
/// 返回是否命中贴边状态（false = 自由态或判定失败），调用方据此决定是否做可见性兜底。
pub fn restore_dock_state(app: &AppHandle) -> bool {
    let Some(ball) = app.get_webview_window(BALL_LABEL) else {
        log::warn!("贴边状态恢复失败：未找到悬浮球窗口");
        return false;
    };
    let Ok(Some(monitor)) = ball.current_monitor() else {
        log::warn!("贴边状态恢复失败：无法获取悬浮球所在显示器（开机早期显示器可能未就绪）");
        return false;
    };
    let Ok(pos) = ball.outer_position() else {
        log::warn!("贴边状态恢复失败：无法读取悬浮球位置");
        return false;
    };
    let Ok(size) = ball.inner_size() else {
        log::warn!("贴边状态恢复失败：无法读取悬浮球尺寸");
        return false;
    };
    let work_rect = monitor.work_area();
    let ball = Rect {
        x: pos.x as f64,
        y: pos.y as f64,
        width: size.width as f64,
        height: size.height as f64,
    };
    let work = Rect {
        x: work_rect.position.x as f64,
        y: work_rect.position.y as f64,
        width: work_rect.size.width as f64,
        height: work_rect.size.height as f64,
    };
    let visible = COLLAPSED_VISIBLE_LOGICAL_PX * monitor.scale_factor();
    // 判定基准是物理像素，容差随 DPI 缩放，缓解高 DPI / 启动期 DPI 抖动导致的漏匹配
    let tol = RESTORE_TOLERANCE_PX * monitor.scale_factor();
    let restored = [DockSide::Left, DockSide::Right, DockSide::Top, DockSide::Bottom]
        .into_iter()
        .find_map(|side| {
            let (cx, cy) = dock_collapsed_pos(side, &work, &ball, visible);
            if (cx - ball.x).abs() <= tol && (cy - ball.y).abs() <= tol {
                return Some((side, true));
            }
            let (ex, ey) = dock_expanded_pos(side, &work, &ball);
            if (ex - ball.x).abs() <= tol && (ey - ball.y).abs() <= tol {
                return Some((side, false));
            }
            None
        });
    match restored {
        Some((side, collapsed)) => {
            set_dock(Some(side), collapsed);
            log::info!("悬浮球贴边状态已恢复：{:?} collapsed={collapsed}", side);
            true
        }
        None => {
            // 自由态属正常情况；打印现场数据便于排查"贴边未恢复 → 球不可见"类问题
            log::info!(
                "悬浮球处于自由态（未匹配贴边位）：pos=({},{}) size={}x{} scale={}",
                ball.x,
                ball.y,
                ball.width,
                ball.height,
                monitor.scale_factor()
            );
            false
        }
    }
}

/// 球矩形在各显示器矩形上的可见面积占比（交集面积/球面积），返回最大值及对应索引。
/// 完全出界返回 (0.0, None)；并列时取先出现的显示器。
fn visible_area_ratio(ball: &Rect, monitors: &[Rect]) -> (f64, Option<usize>) {
    let ball_area = ball.width * ball.height;
    if ball_area <= 0.0 {
        return (0.0, None);
    }
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let ix = (ball.x + ball.width).min(m.x + m.width) - ball.x.max(m.x);
            let iy = (ball.y + ball.height).min(m.y + m.height) - ball.y.max(m.y);
            let ratio = if ix > 0.0 && iy > 0.0 {
                ix * iy / ball_area
            } else {
                0.0
            };
            (ratio, i)
        })
        .fold((0.0, None::<usize>), |best, (ratio, i)| {
            if ratio > best.0 {
                (ratio, Some(i))
            } else {
                best
            }
        })
}

/// 把球钳制进 work area（四周留 margin_px 物理像素边距）；work 比球还小时居中。
fn clamp_into_work_area(ball: &Rect, work: &Rect, margin_px: f64) -> (f64, f64) {
    let clamp_axis = |start: f64, len: f64, area_start: f64, area_len: f64| -> f64 {
        let min = area_start + margin_px;
        let max = area_start + area_len - margin_px - len;
        if max < min {
            area_start + (area_len - len) / 2.0
        } else {
            start.clamp(min, max)
        }
    };
    let x = clamp_axis(ball.x, ball.width, work.x, work.width);
    let y = clamp_axis(ball.y, ball.height, work.y, work.height);
    (x, y)
}

/// 可见性兜底：球在所有显示器上的可见面积都不足半数（显示器拓扑变化、
/// 历史保存位置越界、贴边恢复失败停在收起位等），移回目标显示器 work area 内并落盘。
/// 返回是否发生了移动。命中贴边状态的调用方不应触发本函数（收起位大半出界是设计行为）。
pub fn ensure_ball_visible(app: &AppHandle) -> bool {
    let Some(ball_win) = app.get_webview_window(BALL_LABEL) else {
        return false;
    };
    let Ok(monitors) = ball_win.available_monitors() else {
        log::warn!("悬浮球可见性兜底失败：无法枚举显示器");
        return false;
    };
    if monitors.is_empty() {
        log::warn!("悬浮球可见性兜底失败：显示器列表为空");
        return false;
    }
    let Ok(pos) = ball_win.outer_position() else {
        log::warn!("悬浮球可见性兜底失败：无法读取位置");
        return false;
    };
    let Ok(size) = ball_win.inner_size() else {
        log::warn!("悬浮球可见性兜底失败：无法读取尺寸");
        return false;
    };
    let ball = Rect {
        x: pos.x as f64,
        y: pos.y as f64,
        width: size.width as f64,
        height: size.height as f64,
    };
    let monitor_rects: Vec<Rect> = monitors
        .iter()
        .map(|m| Rect {
            x: m.position().x as f64,
            y: m.position().y as f64,
            width: m.size().width as f64,
            height: m.size().height as f64,
        })
        .collect();
    let (ratio, best) = visible_area_ratio(&ball, &monitor_rects);
    if ratio >= 0.5 {
        return false;
    }
    // 目标显示器：可见交集最大者；完全无交集则主显示器（再退回第一个）
    let target = match best {
        Some(i) => Some(monitors[i].clone()),
        None => ball_win
            .primary_monitor()
            .ok()
            .flatten()
            .or_else(|| monitors.first().cloned()),
    };
    let Some(target) = target else {
        log::warn!("悬浮球可见性兜底失败：无可用显示器");
        return false;
    };
    let work_rect = target.work_area();
    let work = Rect {
        x: work_rect.position.x as f64,
        y: work_rect.position.y as f64,
        width: work_rect.size.width as f64,
        height: work_rect.size.height as f64,
    };
    let (nx, ny) = clamp_into_work_area(&ball, &work, 8.0);
    log::warn!(
        "悬浮球窗口不可见（可见占比 {:.2}），已从 ({},{}) 移回工作区 ({},{})",
        ratio,
        ball.x,
        ball.y,
        nx.round(),
        ny.round()
    );
    let _ = ball_win.set_position(PhysicalPosition::new(
        nx.round() as i32,
        ny.round() as i32,
    ));
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = handle.save_window_state(StateFlags::POSITION);
    });
    true
}

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

/// 启动首显悬浮球（幂等、一次性）：由主窗口页面加载完成或兜底定时竞争触发，仅第一次生效。
/// 延迟到此刻是因为开机登录早期 DPI / 显示器工作区可能尚未就绪，过早执行会让
/// 贴边状态误判、球停在收起位不可见（用户手动开关后恢复正是同一逻辑的延迟重试）。
pub fn startup_show(app: &AppHandle) {
    if BALL_STARTUP_SHOWN.swap(true, Ordering::AcqRel) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || ensure_ball_window(&handle));
}

/// 启动首显兜底定时：主窗口页面加载事件未触发（页面加载异常等）时，
/// 延迟 delay_ms 后仍触发首显。
pub fn schedule_startup_fallback(app: &AppHandle, delay_ms: u64) {
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        startup_show(&handle);
    });
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
        // 按已恢复的窗口位置还原贴边状态（收起/展开/自由）；
        // 未命中时做可见性兜底，防止球停在收起位/出界位不可见
        if !restore_dock_state(app) {
            ensure_ball_visible(app);
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
        // 面板关闭后，贴边展开的球安排延迟收回（点球关面板时鼠标仍在球上，
        // 收回线程会因光标在球内而放弃；真正移开时由 mouseleave 再安排）
        #[cfg(target_os = "windows")]
        if dock_state()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
            && !DOCK_COLLAPSED.load(Ordering::Acquire)
        {
            win32_dock::schedule_recollapse(app);
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
    // 拖动开始：置位拖动标志并使在途贴边动画失效，避免两个 SetWindowPos
    // 循环同时驱动窗口产生抖动
    BALL_DRAGGING.store(true, Ordering::Release);
    DOCK_ANIM_GEN.fetch_add(1, Ordering::AcqRel);
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
        // 拖动结束：清拖动标志后做贴边吸附判定（收起动画 / 清除 dock），
        // 并统一落盘最终位置
        BALL_DRAGGING.store(false, Ordering::Release);
        win32_dock::finish_ball_drag(&app, hwnd);
    });
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
        // zcode / dsh 的 provider 均由应用内自管，cc-switch 不写入/不切换，
        // 悬浮球（快速切换入口）不展示这两类分组
        if matches!(
            app_type,
            crate::app_config::AppType::Zcode | crate::app_config::AppType::Dsh
        ) {
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

    #[test]
    fn snap_none_when_ball_far_from_edges() {
        let ball = rect(500.0, 400.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let monitor = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX), None);
    }

    #[test]
    fn snap_left_when_near_left_edge() {
        // 距左缘 10px < 32px，距右缘远；无任务栏差异
        let ball = rect(10.0, 400.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let monitor = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(
            should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX),
            Some(DockSide::Left)
        );
    }

    #[test]
    fn snap_right_when_ball_crosses_right_edge() {
        // 球越出右缘 1px（右距 = -1，取 abs）；左侧远
        let ball = rect(1865.0, 400.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let monitor = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(
            should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX),
            Some(DockSide::Right)
        );
    }

    #[test]
    fn snap_top_when_near_top_edge() {
        let ball = rect(500.0, 8.0, 56.0, 56.0);
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let monitor = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(
            should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX),
            Some(DockSide::Top)
        );
    }

    #[test]
    fn snap_skips_taskbar_side() {
        // 任务栏在底部：work 底边高于显示器底边，贴底也应被跳过
        let ball = rect(500.0, 1020.0, 56.0, 56.0); // 距 work 底缘 4px
        let work = rect(0.0, 0.0, 1920.0, 1040.0);
        let monitor = rect(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX), None);
    }

    #[test]
    fn snap_uses_closer_side_on_secondary_monitor_with_negative_origin() {
        // 副屏在主屏左侧：work 原点为负；球距左缘 5px
        let ball = rect(-1915.0, 400.0, 56.0, 56.0);
        let work = rect(-1920.0, 0.0, 1920.0, 1040.0);
        let monitor = rect(-1920.0, 0.0, 1920.0, 1040.0);
        assert_eq!(
            should_snap(&ball, &work, &monitor, SNAP_THRESHOLD_PX),
            Some(DockSide::Left)
        );
    }

    #[test]
    fn taskbar_side_detected_from_monitor_work_gap() {
        let full = rect(0.0, 0.0, 1920.0, 1080.0);
        // 底部任务栏
        assert_eq!(taskbar_side(&full, &rect(0.0, 0.0, 1920.0, 1040.0)), Some(DockSide::Bottom));
        // 顶部任务栏
        assert_eq!(taskbar_side(&full, &rect(0.0, 40.0, 1920.0, 1040.0)), Some(DockSide::Top));
        // 左侧任务栏
        assert_eq!(taskbar_side(&full, &rect(60.0, 0.0, 1860.0, 1080.0)), Some(DockSide::Left));
        // 无任务栏（副屏 / 自动隐藏）
        assert_eq!(taskbar_side(&full, &full), None);
    }

    #[test]
    fn dock_positions_horizontal() {
        let work = rect(0.0, 0.0, 1920.0, 1080.0);
        let ball = rect(10.0, 400.0, 56.0, 56.0);
        // 左侧：展开 x=work.x，y 保持；收起露出 15px → x = -41
        assert_eq!(dock_expanded_pos(DockSide::Left, &work, &ball), (0.0, 400.0));
        assert_eq!(
            dock_collapsed_pos(DockSide::Left, &work, &ball, 15.0),
            (-41.0, 400.0)
        );
        // 右侧
        assert_eq!(
            dock_expanded_pos(DockSide::Right, &work, &ball),
            (1920.0 - 56.0, 400.0)
        );
        assert_eq!(
            dock_collapsed_pos(DockSide::Right, &work, &ball, 15.0),
            (1920.0 - 15.0, 400.0)
        );
    }

    #[test]
    fn dock_positions_vertical_secondary_monitor() {
        // 副屏无任务栏，四边可收缩；1.5x 缩放下球 84x84
        let work = rect(2880.0, 0.0, 1920.0, 1040.0);
        let ball = rect(4000.0, 10.0, 84.0, 84.0);
        // 顶部：展开 y=work.y，x 保持；收起露出 15px → y = -69
        assert_eq!(dock_expanded_pos(DockSide::Top, &work, &ball), (4000.0, 0.0));
        assert_eq!(
            dock_collapsed_pos(DockSide::Top, &work, &ball, 15.0),
            (4000.0, -69.0)
        );
        // 底部
        assert_eq!(
            dock_expanded_pos(DockSide::Bottom, &work, &ball),
            (4000.0, 1040.0 - 84.0)
        );
        assert_eq!(
            dock_collapsed_pos(DockSide::Bottom, &work, &ball, 15.0),
            (4000.0, 1040.0 - 15.0)
        );
    }

    #[test]
    fn visible_area_ratio_reports_max_intersection() {
        let primary = rect(0.0, 0.0, 2560.0, 1320.0);
        // 贴顶收起位：70px 高只露 12px 在屏内 → 占比 12/70
        let top_collapsed = rect(2054.0, -58.0, 70.0, 70.0);
        assert!((visible_area_ratio(&top_collapsed, &[primary]).0 - 12.0 / 70.0).abs() < 1e-9);
        // 完全在屏内 → 1.0
        let inside = rect(100.0, 100.0, 70.0, 70.0);
        assert_eq!(visible_area_ratio(&inside, &[primary]), (1.0, Some(0)));
        // 完全出界 → (0.0, None)
        let off = rect(5000.0, 5000.0, 70.0, 70.0);
        assert_eq!(visible_area_ratio(&off, &[primary]), (0.0, None));
        // 双显示器各露一半时取先出现的（并列取最大即可）
        let secondary = rect(2560.0, 0.0, 1920.0, 1080.0);
        let spanning = rect(2525.0, 500.0, 70.0, 70.0);
        let (ratio, best) = visible_area_ratio(&spanning, &[primary, secondary]);
        assert!((ratio - 0.5).abs() < 1e-9);
        assert_eq!(best, Some(0));
    }

    #[test]
    fn clamp_into_work_area_pulls_ball_back_inside() {
        let work = rect(0.0, 0.0, 2560.0, 1320.0);
        // 贴顶收起位 y=-58 → 抬回顶部内边（margin=8）
        let ball = rect(2054.0, -58.0, 70.0, 70.0);
        assert_eq!(clamp_into_work_area(&ball, &work, 8.0), (2054.0, 8.0));
        // 完全出右界 → 拉回右侧内边
        let off_right = rect(3000.0, 100.0, 70.0, 70.0);
        assert_eq!(
            clamp_into_work_area(&off_right, &work, 8.0),
            (2560.0 - 8.0 - 70.0, 100.0)
        );
        // work 比球还小 → 居中（两侧对称出界）
        let tiny = rect(0.0, 0.0, 40.0, 40.0);
        let big = rect(-100.0, -100.0, 70.0, 70.0);
        assert_eq!(clamp_into_work_area(&big, &tiny, 8.0), (-15.0, -15.0));
    }
}
