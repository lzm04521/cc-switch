import { describe, expect, it } from "vitest";
import {
  computeTokensPerSecond,
  getCacheWriteAvailability,
} from "@/types/usage";

describe("getCacheWriteAvailability", () => {
  it("distinguishes cache-write support across fixed protocols", () => {
    expect(getCacheWriteAvailability(["claude"])).toBe("ok");
    expect(getCacheWriteAvailability(["pi"])).toBe("partial");
    expect(getCacheWriteAvailability(["codex", "gemini"])).toBe("na");
    expect(getCacheWriteAvailability(["claude", "codex"])).toBe("partial");
    expect(getCacheWriteAvailability([])).toBe("ok");
  });
});

describe("computeTokensPerSecond", () => {
  const base = {
    dataSource: "proxy",
    isStreaming: true,
    firstTokenMs: 3000,
    latencyMs: 53000,
    outputTokens: 1000,
  };

  it("computes speed over the pure generation window (latency - first token)", () => {
    // gen = 50000ms → 1000 tok / 50s = 20 t/s
    expect(computeTokensPerSecond(base)).toBeCloseTo(20, 9);
  });

  it("returns null for non-proxy data sources", () => {
    expect(
      computeTokensPerSecond({ ...base, dataSource: "session_log" }),
    ).toBeNull();
    expect(
      computeTokensPerSecond({ ...base, dataSource: "codex_session" }),
    ).toBeNull();
  });

  it("treats a missing dataSource as proxy-direct", () => {
    expect(
      computeTokensPerSecond({ ...base, dataSource: undefined }),
    ).toBeCloseTo(20, 9);
  });

  it("returns null for non-streaming rows", () => {
    expect(computeTokensPerSecond({ ...base, isStreaming: false })).toBeNull();
  });

  it("returns null when first-token timing is absent", () => {
    expect(
      computeTokensPerSecond({ ...base, firstTokenMs: undefined }),
    ).toBeNull();
  });

  it("returns null when latency does not exceed first-token time", () => {
    expect(computeTokensPerSecond({ ...base, latencyMs: 3000 })).toBeNull();
    expect(computeTokensPerSecond({ ...base, latencyMs: 2000 })).toBeNull();
  });

  it("returns null for zero output tokens", () => {
    expect(computeTokensPerSecond({ ...base, outputTokens: 0 })).toBeNull();
  });
});
