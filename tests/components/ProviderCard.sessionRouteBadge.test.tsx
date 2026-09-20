import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider, Settings } from "@/types";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => null,
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: () => null,
}));

vi.mock("@/components/UsageFooter", () => ({ default: () => null }));
vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  default: () => null,
}));
vi.mock("@/components/CopilotQuotaFooter", () => ({ default: () => null }));
vi.mock("@/components/CodexOauthQuotaFooter", () => ({
  default: () => null,
}));
vi.mock("@/components/XaiOauthQuotaFooter", () => ({ default: () => null }));

vi.mock("@/lib/query/failover", () => ({
  useProviderHealth: () => ({ data: undefined }),
}));

// settings 数据由用例按需覆盖（routePrefix 三态：未设置 / 自定义 / 非法）
const settingsData = vi.hoisted(() => ({
  current: undefined as Settings | undefined,
}));

vi.mock("@/lib/query/queries", () => ({
  useUsageQuery: () => ({ data: undefined }),
  useSettingsQuery: () => ({ data: settingsData.current }),
}));

const sessionRouteProvider: Provider = {
  id: "session-route-provider",
  name: "Session Route Upstream",
  settingsConfig: {},
  // apiFormat 缺省 = Codex 原生 Responses 直连，原本不需要路由；
  // route_enabled 短路让「需要路由」也亮起。
  meta: { route_enabled: true, route_key: "BMAX" },
};

function renderCard(provider: Provider, appId: "codex" | "claude" = "codex") {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <ProviderCard
        provider={provider}
        appId={appId}
        isCurrent={false}
        isProxyRunning={false}
        onSwitch={vi.fn()}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        onConfigureUsage={vi.fn()}
        onOpenWebsite={vi.fn()}
        onDuplicate={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

describe("ProviderCard 会话路由 badges", () => {
  beforeEach(() => {
    settingsData.current = undefined;
  });

  it("route_enabled 让原本直连的供应商同时显示「需要路由」与绿色「会话路由」Tag，且顺序在后", () => {
    renderCard(sessionRouteProvider);

    const needsRouting = screen.getByText("需要路由");
    const sessionRoute = screen.getByText("会话路由：G.BMAX");

    expect(needsRouting).toBeInTheDocument();
    expect(sessionRoute).toBeInTheDocument();
    // 「会话路由」始终紧跟「需要路由」之后。
    expect(
      needsRouting.compareDocumentPosition(sessionRoute),
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    // 绿色 = ProviderStatusBadge success tone（emerald）。
    expect(sessionRoute).toHaveClass("bg-emerald-100", "text-emerald-700");
  });

  it("前缀读取「会话级路由前缀」设置拼接（自定义 @）", () => {
    settingsData.current = { routePrefix: "@" } as Settings;
    renderCard(sessionRouteProvider);

    expect(screen.getByText("会话路由：@BMAX")).toBeInTheDocument();
    expect(screen.queryByText(/G\.BMAX/)).not.toBeInTheDocument();
  });

  it("设置的非法前缀回退默认 G.（与后端 normalize_route_prefix 一致）", () => {
    // "G" 结尾为字母数字，无边界符，非法
    settingsData.current = { routePrefix: "G" } as Settings;
    renderCard(sessionRouteProvider);

    expect(screen.getByText("会话路由：G.BMAX")).toBeInTheDocument();
  });

  it("未开启会话路由时不显示「会话路由」Tag，直连供应商也不显示「需要路由」", () => {
    renderCard({
      id: "direct-provider",
      name: "Direct Upstream",
      settingsConfig: {},
      meta: { route_key: "BMAX" },
    });

    expect(screen.queryByText(/会话路由/)).not.toBeInTheDocument();
    expect(screen.queryByText("需要路由")).not.toBeInTheDocument();
  });
});
