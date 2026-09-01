import { describe, expect, it } from "vitest";
import { aggregateSummaries } from "@/components/usage/UsageHero";
import { formatUsageDataSummary } from "@/utils/usageDisplay";
import type { UsageSummary } from "@/types/usage";

const labels = {
  invalid: "Invalid",
  remaining: "Remaining:",
  used: "Used:",
};

describe("formatUsageDataSummary", () => {
  it("formats used percentage when remaining is omitted", () => {
    expect(
      formatUsageDataSummary(
        {
          planName: "Coco OpenRouter",
          used: 55,
          total: 100,
          unit: "%",
        },
        labels,
      ),
    ).toBe("[Coco OpenRouter] Used: 55%");
  });

  it("formats remaining when present", () => {
    expect(
      formatUsageDataSummary(
        {
          planName: "Balance",
          remaining: 12.5,
          unit: "USD",
        },
        labels,
      ),
    ).toBe("[Balance] Remaining: 12.50 USD");
  });

  it("formats invalid results without requiring quota fields", () => {
    expect(
      formatUsageDataSummary(
        {
          isValid: false,
          invalidMessage: "Unauthorized",
        },
        labels,
      ),
    ).toBe("Unauthorized");
  });
});

describe("aggregateSummaries", () => {
  const appSummary = (
    overrides: Partial<UsageSummary>,
  ): UsageSummary => ({
    totalRequests: 0,
    totalCost: "0",
    totalInputTokens: 0,
    totalOutputTokens: 0,
    totalCacheCreationTokens: 0,
    totalCacheReadTokens: 0,
    successRate: 0,
    realTotalTokens: 0,
    cacheHitRate: 0,
    ...overrides,
  });

  it("re-divides avg t/s from summed numerator/denominator, not arithmetic mean", () => {
    // claude: 1000 tok / 50s = 20 t/s；codex: 100 tok / 10s = 10 t/s
    // 加权 = 1100 tok / 60s ≈ 18.33（算术平均会是 15）
    const merged = aggregateSummaries([
      appSummary({ streamOutputTokens: 1000, streamGenMs: 50_000 }),
      appSummary({ streamOutputTokens: 100, streamGenMs: 10_000 }),
    ]);
    expect(merged.streamOutputTokens).toBe(1100);
    expect(merged.streamGenMs).toBe(60_000);
    expect(merged.avgTokensPerSecond).toBeCloseTo(
      (1100 * 1000) / 60_000,
      9,
    );
  });

  it("returns null avg when no app has computable rows", () => {
    const merged = aggregateSummaries([
      appSummary({}),
      appSummary({ streamOutputTokens: 0, streamGenMs: 0 }),
    ]);
    expect(merged.avgTokensPerSecond).toBeNull();
  });

  it("treats missing stream fields as zero (old backend payloads)", () => {
    const merged = aggregateSummaries([
      appSummary({ streamOutputTokens: 500, streamGenMs: 25_000 }),
      appSummary({}),
    ]);
    expect(merged.streamOutputTokens).toBe(500);
    expect(merged.avgTokensPerSecond).toBeCloseTo(20, 9);
  });
});
