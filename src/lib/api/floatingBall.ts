import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import type { UsageResult } from "@/types";
import type { AppId } from "./types";

export interface BallProviderInfo {
  id: string;
  name: string;
  icon?: string | null;
  iconColor?: string | null;
}

export interface BallSection {
  appType: AppId;
  currentProviderId?: string | null;
  providers: BallProviderInfo[];
}

/** 后端 UsageCache 全量快照条目（get_provider_usage_cache 返回） */
export interface UsageCacheSnapshot {
  appType: AppId;
  providerId: string;
  result: UsageResult;
  /** 查询时刻（毫秒时间戳），面板显示"x 分钟前"用 */
  queriedAt: number;
}

export const FLOATING_BALL_SECTIONS_KEY = ["floating-ball-sections"] as const;
export const PROVIDER_USAGE_CACHE_KEY = ["provider-usage-cache"] as const;

export const floatingBallApi = {
  togglePanel: () => invoke<"opened" | "closed">("toggle_ball_panel"),
  hidePanel: () => invoke<boolean>("hide_ball_panel"),
  onPanelBlur: () => invoke<boolean>("on_ball_panel_blur"),
  savePosition: () => invoke<boolean>("save_ball_position"),
  startDrag: () => invoke<boolean>("start_ball_drag"),
  onHover: (entered: boolean) => invoke<boolean>("on_ball_hover", { entered }),
  setEnabled: (enabled: boolean) =>
    invoke<boolean>("set_floating_ball_enabled", { enabled }),
  getSections: () => invoke<BallSection[]>("get_floating_ball_sections"),
  getUsageCache: () => invoke<UsageCacheSnapshot[]>("get_provider_usage_cache"),
  showMainWindow: () => invoke<boolean>("show_main_window"),
};

export function useFloatingBallSections() {
  return useQuery({
    queryKey: FLOATING_BALL_SECTIONS_KEY,
    queryFn: () => floatingBallApi.getSections(),
  });
}

/** 后端用量缓存快照（只读，不发网络查询；数据由主窗口/托盘触发的查询写穿） */
export function useProviderUsageCache() {
  return useQuery({
    queryKey: PROVIDER_USAGE_CACHE_KEY,
    queryFn: () => floatingBallApi.getUsageCache(),
  });
}
