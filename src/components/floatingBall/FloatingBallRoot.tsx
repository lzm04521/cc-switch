import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { BallIcon } from "./BallIcon";
import { PanelList } from "./PanelList";

/**
 * 悬浮球双窗口共用根组件：
 * - ball 窗口渲染球体（BallIcon）
 * - panel 窗口渲染分组列表（PanelList）
 */
export function FloatingBallRoot() {
  const label = getCurrentWindow().label;

  // ball/panel 均为 transparent 窗口，但全局样式中 body 有背景色，会把窗口
  // 透明区域染成白色（表现为图标周围一圈白边）。显式置为透明，
  // 只让图标 / 面板自身渲染内容。
  useEffect(() => {
    document.documentElement.style.backgroundColor = "transparent";
    document.body.style.backgroundColor = "transparent";
  }, []);

  return label === "panel" ? <PanelList /> : <BallIcon />;
}
