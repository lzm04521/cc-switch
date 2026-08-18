// jsdom 25 未实现 PointerEvent，@testing-library 的 fireEvent 会生成丢失
// 全部属性的事件对象（button/screenX 为 undefined）。先打 Polyfill，
// 让 fireEvent.pointerDown/Up 正确构造事件。
if (typeof window !== "undefined" && !(window as any).PointerEvent) {
  (window as any).PointerEvent = class PointerEventPolyfill extends MouseEvent {
    pointerId: number;
    constructor(type: string, params: PointerEventInit = {}) {
      super(type, params);
      this.pointerId = params.pointerId ?? 0;
    }
  };
}

import { render, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";

const startDragging = vi.fn().mockResolvedValue(undefined);
const onMoved = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "ball",
    startDragging,
    onMoved,
  }),
}));

const togglePanel = vi.fn().mockResolvedValue("opened");
const startDrag = vi.fn().mockResolvedValue(true);
const onHover = vi.fn().mockResolvedValue(true);
vi.mock("@/lib/api/floatingBall", () => ({
  floatingBallApi: {
    togglePanel: () => togglePanel(),
    startDrag: () => startDrag(),
    onHover: (entered: boolean) => onHover(entered),
    savePosition: vi.fn().mockResolvedValue(true),
  },
}));

import { BallIcon } from "./BallIcon";

beforeEach(() => {
  togglePanel.mockClear();
  startDrag.mockClear();
  onHover.mockClear();
  startDragging.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("BallIcon", () => {
  it("按下后未移动即松开（点击）触发 togglePanel，不进入拖动", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      screenX: 10,
      screenY: 10,
    });
    fireEvent.pointerUp(el, { pointerId: 1, screenX: 12, screenY: 11 });
    expect(togglePanel).toHaveBeenCalledTimes(1);
    expect(startDrag).not.toHaveBeenCalled();
    expect(startDragging).not.toHaveBeenCalled();
  });

  it("移动超过阈值交给后端原生循环拖动（只触发一次），松手不触发 togglePanel", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      screenX: 10,
      screenY: 10,
    });
    // 位移 sqrt(6^2+2^2) ≈ 6.3 > 4 → 进入拖动，调用后端 start_ball_drag
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 16, screenY: 12, buttons: 1 });
    expect(startDrag).toHaveBeenCalledTimes(1);
    expect(startDragging).not.toHaveBeenCalled();
    // 后续 move 不再重复触发（前端已退出本次手势）
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 100, screenY: 100, buttons: 1 });
    expect(startDrag).toHaveBeenCalledTimes(1);
    fireEvent.pointerUp(el, { pointerId: 1, screenX: 210, screenY: 10 });
    expect(togglePanel).not.toHaveBeenCalled();
  });

  it("位移未达拖动阈值（且未达点击阈值）的移动不进入拖动，松手仍为点击", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      screenX: 10,
      screenY: 10,
    });
    // 位移 3.6px：> 0 但 < 拖动阈值 4px → 不进入拖动
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 13, screenY: 12, buttons: 1 });
    fireEvent.pointerUp(el, { pointerId: 1, screenX: 13, screenY: 12 });
    expect(togglePanel).toHaveBeenCalledTimes(1); // 位移 3.6px < 5 → 点击
    expect(startDrag).not.toHaveBeenCalled();
    expect(startDragging).not.toHaveBeenCalled();
  });

  it("mouseenter/mouseleave 上报 on_ball_hover（贴边展开/收回联动）", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.mouseEnter(el);
    expect(onHover).toHaveBeenCalledWith(true);
    fireEvent.mouseLeave(el);
    expect(onHover).toHaveBeenCalledWith(false);
  });

  it("拖动中触发的 mouseleave 仍上报（后端 BALL_DRAGGING 校验负责拦截）", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      screenX: 10,
      screenY: 10,
    });
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 16, screenY: 12, buttons: 1 }); // 进入拖动
    fireEvent.mouseLeave(el); // 拖动中球被移走触发的 leave
    expect(onHover).toHaveBeenCalledWith(false);
  });

  it("pointerup 丢失后，无按键的 pointermove 自愈清理且不误触发 startDrag", () => {
    const { container } = render(<BallIcon />);
    const el = container.firstChild as HTMLElement;
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      screenX: 10,
      screenY: 10,
    });
    // 进入拖动（后端接管，pointerup 因球被移走而丢失）
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 16, screenY: 12, buttons: 1 });
    expect(startDrag).toHaveBeenCalledTimes(1);
    // 左键已松开（buttons=0）仍收到 move：应自愈清理，而不是用残留 down
    // 再次判定为拖动
    fireEvent.pointerMove(el, { pointerId: 1, screenX: 100, screenY: 100, buttons: 0 });
    expect(startDrag).toHaveBeenCalledTimes(1);
    // 清理后 hover 不再被残留 dragging 抑制
    fireEvent.mouseEnter(el);
    expect(onHover).toHaveBeenCalledWith(true);
  });
});
