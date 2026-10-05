import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import { buildSwitchSections } from "@/components/providers/presentation";
import type { Provider } from "@/types";

// fork 定制：route_enabled 供应商的卡片 chip（v4 迁入 presentation 层）
const t = ((key: string, opts?: Record<string, unknown>) =>
  opts
    ? `${key}:${JSON.stringify(opts)}`
    : key) as unknown as TFunction;

const sessionRouteProvider: Provider = {
  id: "session-route-provider",
  name: "Session Route Upstream",
  settingsConfig: {},
  // apiFormat 缺省 = Codex 原生 Responses 直连，原本不需要路由；
  // route_enabled 短路让「需要路由」也亮起。
  meta: { route_enabled: true, route_key: "BMAX" },
};

const plainProvider: Provider = {
  id: "plain-provider",
  name: "Plain Upstream",
  settingsConfig: {},
};

function chipsOf(providers: Provider[], routePrefix?: string) {
  const sections = buildSwitchSections({
    app: "codex",
    t,
    providers,
    active: "direct",
    view: "direct",
    directId: null,
    routeId: null,
    failoverOn: false,
    queue: [],
    stackMembers: new Map(),
    routingReason: () => "",
    routePrefix,
    serviceRunning: false,
    actions: {
      switchDirect: () => undefined,
      needsRouteDialog: () => undefined,
      exitAndUse: () => undefined,
      routeTo: () => undefined,
      queueAdd: () => undefined,
      queueRemove: () => undefined,
      queueMove: () => undefined,
      stackAdd: () => undefined,
      stackRemove: () => undefined,
      stackSetDefault: () => undefined,
    },
  });
  return sections.flatMap((section) =>
    section.items.map((item) => item.presentation.chips.map((c) => c.key)),
  )[0] ?? [];
}

describe("会话路由 chip（presentation 层）", () => {
  it("route_enabled 供应商显示 sessionRoute chip，且在 needsRoute 之后", () => {
    const chips = chipsOf([sessionRouteProvider]);
    expect(chips).toContain("needsRoute");
    expect(chips).toContain("sessionRoute");
    expect(chips.indexOf("sessionRoute")).toBeGreaterThan(
      chips.indexOf("needsRoute"),
    );
  });

  it("前缀缺省回退 G.，自定义前缀透传拼接", () => {
    const sections = buildSwitchSections({
      app: "codex",
      t,
      providers: [sessionRouteProvider],
      active: "direct",
      view: "direct",
      directId: null,
      routeId: null,
      failoverOn: false,
      queue: [],
      stackMembers: new Map(),
      routingReason: () => "",
      routePrefix: "@",
      serviceRunning: false,
      actions: {
        switchDirect: () => undefined,
        needsRouteDialog: () => undefined,
        exitAndUse: () => undefined,
        routeTo: () => undefined,
        queueAdd: () => undefined,
        queueRemove: () => undefined,
        queueMove: () => undefined,
        stackAdd: () => undefined,
        stackRemove: () => undefined,
        stackSetDefault: () => undefined,
      },
    });
    const chip = sections
      .flatMap((s) => s.items)
      .flatMap((i) => i.presentation.chips)
      .find((c) => c.key === "sessionRoute");
    // mock t 不做 i18next 插值，断言透传的插值参数即可
    expect(chip?.label).toContain('"prefix":"@"');
    expect(chip?.label).toContain('"routeKey":"BMAX"');
    expect(chip?.tone).toBe("success");
  });

  it("未开启会话路由的供应商不显示 sessionRoute chip", () => {
    const chips = chipsOf([plainProvider]);
    expect(chips).not.toContain("sessionRoute");
  });
});
