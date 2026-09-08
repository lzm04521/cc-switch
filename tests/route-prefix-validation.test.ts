import { describe, expect, it } from "vitest";
import {
  routePrefixLikelyConflicts,
  validateRoutePrefixValue,
} from "@/lib/routePrefix";

describe("validateRoutePrefixValue", () => {
  it("接受 G. / @ / ## 等带边界符前缀", () => {
    expect(validateRoutePrefixValue("G.")).toEqual({ ok: true });
    expect(validateRoutePrefixValue("@")).toEqual({ ok: true });
    expect(validateRoutePrefixValue("##")).toEqual({ ok: true });
  });
  it("拒绝空值 / 超长 / 非法字符", () => {
    expect(validateRoutePrefixValue("")).toEqual({
      ok: false,
      reason: "empty",
    });
    expect(validateRoutePrefixValue("  ")).toEqual({
      ok: false,
      reason: "empty",
    });
    expect(validateRoutePrefixValue("toolongpfx.")).toEqual({
      ok: false,
      reason: "length",
    });
    expect(validateRoutePrefixValue("路.")).toEqual({
      ok: false,
      reason: "charset",
    });
  });
  it("拒绝冒号与裸字母数字结尾", () => {
    expect(validateRoutePrefixValue("G.:")).toEqual({
      ok: false,
      reason: "colon",
    });
    expect(validateRoutePrefixValue("G")).toEqual({
      ok: false,
      reason: "boundary",
    });
    expect(validateRoutePrefixValue("go2")).toEqual({
      ok: false,
      reason: "boundary",
    });
  });
});

describe("routePrefixLikelyConflicts", () => {
  it("识别与模型名族重合的前缀", () => {
    expect(routePrefixLikelyConflicts("claude.")).toBe(true);
    expect(routePrefixLikelyConflicts("gpt.")).toBe(true);
    expect(routePrefixLikelyConflicts("G.")).toBe(false);
    expect(routePrefixLikelyConflicts("@")).toBe(false);
  });
});
