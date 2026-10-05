import { describe, expect, it } from "vitest";
import type { TFunction } from "i18next";
import {
  buildSwitchSections,
  type SwitchModeInput,
} from "@/components/providers/presentation";
import type { Provider } from "@/types";

// fork 定制：会话路由 chip 只挂路由视图（含队列卡），tone=route；直连视图只剩「需要路由」
const t = ((key: string, opts?: Record<string, unknown>) =>
  opts ? `${key}:${JSON.stringify(opts)}` : key) as unknown as TFunction;

const sessionRouteProvider: Provider = {
  id: "session-route-provider",
  name: "Session Route Upstream",
  settingsConfig: {},
  // apiFormat 缺省 = Codex 原生 Responses 直连，原本不需要路由；
  // route_enabled 短路让「需要路由」在直连视图也亮起。
  meta: { route_enabled: true, route_key: "BMAX" },
};

const plainProvider: Provider = {
  id: "plain-provider",
  name: "Plain Upstream",
  settingsConfig: {},
};

// Claude 官方订阅：路由视图里是 blocked 卡；route_enabled 属脏数据
const blockedDirtyProvider: Provider = {
  id: "blocked-dirty",
  name: "Blocked Dirty",
  category: "official",
  settingsConfig: {},
  meta: { route_enabled: true, route_key: "DIRTY" },
};

function buildSections(overrides: Partial<SwitchModeInput> = {}) {
  const input: SwitchModeInput = {
    app: "codex",
    t,
    providers: [sessionRouteProvider, plainProvider],
    active: "route",
    view: "route",
    directId: null,
    routeId: null,
    failoverOn: false,
    queue: [],
    stackMembers: new Map(),
    routingReason: () => "",
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
    ...overrides,
  };
  return buildSwitchSections(input);
}

function chipsOf(
  sections: ReturnType<typeof buildSections>,
  id: string,
): { key: string; label: string; tone?: string }[] {
  for (const section of sections) {
    const found = section.items.find((entry) => entry.provider.id === id);
    if (found) return found.presentation.chips;
  }
  throw new Error(`no card for ${id}`);
}

describe("会话路由 chip（路由视图专用，fork）", () => {
  it("direct view has no session route chip (needsRoute stays)", () => {
    const sections = buildSections({ active: "direct", view: "direct" });
    const keys = chipsOf(sections, sessionRouteProvider.id).map((c) => c.key);
    expect(keys).toContain("needsRoute");
    expect(keys).not.toContain("sessionRoute");
  });

  it("route view single section shows chip with route tone and default G. prefix", () => {
    const sections = buildSections();
    const chip = chipsOf(sections, sessionRouteProvider.id).find(
      (c) => c.key === "sessionRoute",
    );
    expect(chip?.tone).toBe("route");
    // mock t 不做 i18next 插值，断言透传的插值参数即可
    expect(chip?.label).toContain('"prefix":"G."');
    expect(chip?.label).toContain('"routeKey":"BMAX"');
  });

  it("route view chip carries custom route prefix", () => {
    const sections = buildSections({ routePrefix: "@" });
    const chip = chipsOf(sections, sessionRouteProvider.id).find(
      (c) => c.key === "sessionRoute",
    );
    expect(chip?.tone).toBe("route");
    expect(chip?.label).toContain('"prefix":"@"');
  });

  it("route view failover queue card shows chip alongside priority", () => {
    const sections = buildSections({
      failoverOn: true,
      queue: [sessionRouteProvider.id, plainProvider.id],
      routeId: sessionRouteProvider.id,
    });
    const keys = chipsOf(sections, sessionRouteProvider.id).map((c) => c.key);
    expect(keys).toContain("priority");
    expect(keys).toContain("sessionRoute");
  });

  it("route view rest section shows chip", () => {
    const sections = buildSections({
      failoverOn: true,
      queue: [plainProvider.id],
      routeId: plainProvider.id,
    });
    expect(
      chipsOf(sections, sessionRouteProvider.id).map((c) => c.key),
    ).toContain("sessionRoute");
  });

  it("viewing route while direct still shows chip", () => {
    const sections = buildSections({ active: "direct" });
    expect(
      chipsOf(sections, sessionRouteProvider.id).map((c) => c.key),
    ).toContain("sessionRoute");
  });

  it("blocked variants never show chip (route_enabled dirty data)", () => {
    const overrides = {
      app: "claude" as const,
      providers: [blockedDirtyProvider, sessionRouteProvider],
    };
    // active=route 无故障转移：blocked 卡走 blocked() 变体
    expect(
      chipsOf(buildSections(overrides), blockedDirtyProvider.id).map(
        (c) => c.key,
      ),
    ).not.toContain("sessionRoute");
    // 非 active 分支的 blocked 卡同样不挂
    expect(
      chipsOf(buildSections({ ...overrides, active: "direct" }), blockedDirtyProvider.id).map(
        (c) => c.key,
      ),
    ).not.toContain("sessionRoute");
  });

  it("未开启会话路由的供应商在路由视图不显示 sessionRoute chip", () => {
    const sections = buildSections();
    expect(
      chipsOf(sections, plainProvider.id).map((c) => c.key),
    ).not.toContain("sessionRoute");
  });
});
