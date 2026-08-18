import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
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

export const FLOATING_BALL_SECTIONS_KEY = ["floating-ball-sections"] as const;

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
  showMainWindow: () => invoke<boolean>("show_main_window"),
};

export function useFloatingBallSections() {
  return useQuery({
    queryKey: FLOATING_BALL_SECTIONS_KEY,
    queryFn: () => floatingBallApi.getSections(),
  });
}
