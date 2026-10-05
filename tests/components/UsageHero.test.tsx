import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageHero } from "@/components/usage/UsageHero";
import type { UsageSummary } from "@/types/usage";

// 第 5 张「输出速度」卡（fork）：值来自 summary.avgTokensPerSecond 的加权平均。
// aggregateSummaries 的分子分母重除已有单测（tests/utils/usageDisplay.test.ts），这里只测渲染。
const useSummaryByAppMock = vi.hoisted(() => vi.fn());

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: unknown) => {
      if (typeof options === "string") return options;
      if (options && typeof options === "object") {
        const values = Object.entries(options as Record<string, unknown>)
          .filter(([name]) => name !== "defaultValue")
          .map(([, value]) => String(value));
        return values.length ? `${key}:${values.join(",")}` : key;
      }
      return key;
    },
    i18n: { resolvedLanguage: "en", language: "en" },
  }),
}));

vi.mock("@/lib/query/usage", () => ({
  useUsageSummaryByApp: (...args: unknown[]) => useSummaryByAppMock(...args),
}));

const summary = (overrides: Partial<UsageSummary> = {}): UsageSummary => ({
  totalRequests: 10,
  totalCost: "1.000000",
  totalInputTokens: 100,
  totalOutputTokens: 100,
  totalCacheCreationTokens: 0,
  totalCacheReadTokens: 0,
  successRate: 100,
  realTotalTokens: 200,
  cacheHitRate: 0,
  ...overrides,
});

function renderHero(props: Record<string, unknown> = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <UsageHero range={{ preset: "today" }} refreshIntervalMs={0} {...props} />
    </QueryClientProvider>,
  );
}

function speedCard() {
  const label = screen.getByText("usage.outputSpeed");
  // MetricCard 根节点：label span → label 行 div → 卡片 div
  return label.parentElement!.parentElement!;
}

describe("UsageHero output speed card (fork)", () => {
  beforeEach(() => {
    useSummaryByAppMock.mockReset().mockReturnValue({
      // 指定 appType 时 pickSummary 原样取该行 summary，avg 不被重除
      data: [{ appType: "claude", summary: summary() }],
      isLoading: false,
    });
  });

  it("renders output speed card with weighted tps", () => {
    useSummaryByAppMock.mockReturnValue({
      data: [
        { appType: "claude", summary: summary({ avgTokensPerSecond: 48.6 }) },
      ],
      isLoading: false,
    });
    renderHero({ appType: "claude" });

    expect(within(speedCard()).getByText("49")).toBeInTheDocument();
  });

  it("output speed card shows dash when null", () => {
    useSummaryByAppMock.mockReturnValue({
      data: [
        { appType: "claude", summary: summary({ avgTokensPerSecond: null }) },
      ],
      isLoading: false,
    });
    renderHero({ appType: "claude" });

    expect(within(speedCard()).getByText("--")).toBeInTheDocument();
  });

  it("five metric cards in one row when not compact", () => {
    renderHero({ appType: "claude" });

    // 主指标网格（「更多指标」收起时唯一可见的 grid）
    const grid = document.querySelector("section .grid-cols-5");
    expect(grid).not.toBeNull();
    expect(grid!.children.length).toBe(5);
    expect(screen.getByText("usage.outputSpeed")).toBeInTheDocument();
  });
});
