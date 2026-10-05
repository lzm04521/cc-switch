import { describe, expect, it } from "vitest";
import {
  DEFAULT_VISIBLE_APPS,
  isAdditiveAppId,
  resolveSidebarApps,
} from "@/config/appConfig";
import type { VisibleApps } from "@/types";

describe("appConfig provider lifecycle", () => {
  it.each(["opencode", "openclaw", "hermes", "pi", "mcode"])(
    "classifies %s as additive",
    (appId) => {
      expect(isAdditiveAppId(appId)).toBe(true);
    },
  );

  it.each(["claude", "claude-desktop", "codex", "gemini", "grokbuild"])(
    "does not classify %s as additive",
    (appId) => {
      expect(isAdditiveAppId(appId)).toBe(false);
    },
  );
});

describe("resolveSidebarApps", () => {
  it("returns defaults when settings is undefined or null", () => {
    expect(resolveSidebarApps(undefined)).toEqual(DEFAULT_VISIBLE_APPS);
    expect(resolveSidebarApps(null)).toEqual(DEFAULT_VISIBLE_APPS);
  });

  it("falls back per app to visibleApps when sidebarApps is unset", () => {
    // 稀疏对象模拟旧库 settings：只存了部分键
    const settings = {
      visibleApps: { gemini: false, zcode: true } as VisibleApps,
    };
    const resolved = resolveSidebarApps(settings);
    expect(resolved.gemini).toBe(false);
    // zcode 只出现在 visibleApps → 逐 app 兜底跟随 visibleApps
    expect(resolved.zcode).toBe(true);
    // claude 两处都未出现 → 回落默认值
    expect(resolved.claude).toBe(true);
  });

  it("prefers sidebarApps over visibleApps per app", () => {
    const settings = {
      visibleApps: { gemini: false, codex: false } as VisibleApps,
      sidebarApps: { gemini: true } as VisibleApps,
    };
    const resolved = resolveSidebarApps(settings);
    // sidebarApps 优先
    expect(resolved.gemini).toBe(true);
    // sidebarApps 未覆盖 codex → 回落 visibleApps 的 false
    expect(resolved.codex).toBe(false);
    // 两对象都未覆盖 claude → 回落默认值
    expect(resolved.claude).toBe(true);
  });
});
