import { describe, expect, it } from "vitest";
import {
  normalizeStackKey,
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

describe("normalizeStackKey（fork 五项优化：与后端 stack::slug 同规则）", () => {
  it("保留大小写；「.」与其他非法字符换成横线、连续横线合并、首尾去掉", () => {
    expect(normalizeStackKey("My.Key")).toBe("My-Key");
    expect(normalizeStackKey("a--b")).toBe("a-b");
    expect(normalizeStackKey("A_B")).toBe("A-B");
    expect(normalizeStackKey("--edge--")).toBe("edge");
    expect(normalizeStackKey("Zhipu GLM")).toBe("Zhipu-GLM");
    expect(normalizeStackKey("智谱")).toBe("");
  });
  it("超长截断到 24 位", () => {
    expect(normalizeStackKey("x".repeat(40))).toBe("x".repeat(24));
  });
  it("合法输入原样返回（预览不出现假提示）", () => {
    expect(normalizeStackKey("Zhipu")).toBe("Zhipu");
    expect(normalizeStackKey("zhipu-2")).toBe("zhipu-2");
  });
});
