import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider } from "@/types";
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

vi.mock("@/lib/query/queries", () => ({
  useUsageQuery: () => ({ data: undefined }),
}));

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
  it("route_enabled 让原本直连的供应商同时显示「需要路由」与绿色「会话路由」Tag，且顺序在后", () => {
    // apiFormat 缺省 = Codex 原生 Responses 直连，原本不需要路由。
    renderCard({
      id: "session-route-provider",
      name: "Session Route Upstream",
      settingsConfig: {},
      meta: { route_enabled: true, route_key: "BMAX" },
    });

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
