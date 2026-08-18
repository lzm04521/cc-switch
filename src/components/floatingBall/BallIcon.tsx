import { useEffect, useMemo, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { floatingBallApi } from "@/lib/api/floatingBall";
import { isClickGesture, type Point } from "./utils";
import appIcon from "@/assets/icons/app-icon.png";

/** 位移达到该值（逻辑像素）即判定进入拖动；略小于点击判定阈值 5px，保证"按下即拖" */
const DRAG_THRESHOLD_PX = 4;

/**
 * 悬浮球球体（ball 窗口内渲染）——对齐 PixPin/Snipaste 的悬浮球手感：
 * - 按下后一旦移动超过阈值（4px）即进入拖动，由后端原生循环驱动
 *   （start_ball_drag：Windows 上后台线程轮询光标 + SetWindowPos，
 *   球体全程显示、跟手零延迟）
 * - 按下后未移动（或位移 < 点击阈值 5px）即松开 = 点击 → togglePanel
 * - 拖动结束后保存位置（后端落盘 + 本组件 onMoved 防抖兜底）
 *
 * 前端只负责手势判定，不做逐帧移动：系统 move loop（startDragging）对
 * transparent 分层窗口只画拖动外框（球不动），前端 pointermove + setPosition
 * 逐帧 IPC 驱动又受调度抖动影响会卡顿，两者都已在真机验证中排除。
 */
export function BallIcon() {
  const downRef = useRef<Point | null>(null);
  const draggingRef = useRef(false);
  const saveTimerRef = useRef<number | null>(null);
  // getCurrentWindow() 每次返回新实例，用 useMemo 固定引用，
  // 避免重渲染时 effect 依赖变化导致重新订阅 onMoved 并清掉待触发的保存定时器
  const appWindow = useMemo(() => getCurrentWindow(), []);

  // 拖拽结束后防抖保存位置
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void appWindow
      .onMoved(() => {
        if (saveTimerRef.current !== null) {
          window.clearTimeout(saveTimerRef.current);
        }
        saveTimerRef.current = window.setTimeout(() => {
          void floatingBallApi.savePosition().catch(() => {
            // 保存失败静默（下次拖动再试）
          });
        }, 400);
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else cleanup = unlisten;
      });
    return () => {
      disposed = true;
      cleanup?.();
      if (saveTimerRef.current !== null) {
        window.clearTimeout(saveTimerRef.current);
      }
    };
  }, [appWindow]);

  // 窗口失焦（如拖动中被 Alt+Tab 打断）时丢弃按下状态，避免残留 down 导致
  // 之后鼠标一移动就误触发拖动
  useEffect(() => {
    const reset = () => {
      downRef.current = null;
      draggingRef.current = false;
    };
    window.addEventListener("blur", reset);
    return () => window.removeEventListener("blur", reset);
  }, []);

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    downRef.current = { x: e.screenX, y: e.screenY };
    draggingRef.current = false;
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    // 左键已松开却仍在 move：说明本次手势的 pointerup 丢失（拖动被后端接管后
    // 球被移走，松手时鼠标已不在球窗口上），自愈清理，避免 down/dragging
    // 残留把后续手势判定和 hover 上报全部卡死
    if ((e.buttons & 1) === 0) {
      downRef.current = null;
      draggingRef.current = false;
      return;
    }
    const down = downRef.current;
    if (!down || draggingRef.current) return;
    if (
      Math.hypot(e.screenX - down.x, e.screenY - down.y) < DRAG_THRESHOLD_PX
    ) {
      return;
    }
    // 进入拖动：立即清空按下状态，本次手势完全交给后端原生循环
    draggingRef.current = true;
    downRef.current = null;
    void floatingBallApi.startDrag().catch((err) => {
      draggingRef.current = false;
      console.error("[BallIcon] startDrag failed", err);
    });
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    const down = downRef.current;
    downRef.current = null;
    if (!down) return;
    // 未进入拖动且位移小于阈值 → 点击
    if (
      !draggingRef.current &&
      isClickGesture(down, { x: e.screenX, y: e.screenY })
    ) {
      void floatingBallApi.togglePanel().catch(() => {});
    }
    draggingRef.current = false;
  };

  const handlePointerCancel = () => {
    downRef.current = null;
    draggingRef.current = false;
  };

  // 贴边隐藏联动：hover 露条 → 后端滑出展开；移开 → 延迟收回。
  // 不在前端做拖动抑制——后端 ball_hover 自带 BALL_DRAGGING / 动画互斥校验
  //（真正的状态源在后端），前端抑制会在 pointerup 丢失时把 hover 永久卡死
  const handleMouseEnter = () => {
    void floatingBallApi.onHover(true).catch(() => {});
  };
  const handleMouseLeave = () => {
    void floatingBallApi.onHover(false).catch(() => {});
  };

  return (
    <div
      className="floating-ball"
      onContextMenu={(e) => e.preventDefault()}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerCancel}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      <img
        src={appIcon}
        alt=""
        draggable={false}
        className="floating-ball-icon"
      />
    </div>
  );
}
