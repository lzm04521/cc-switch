import { describe, expect, it } from "vitest";
import {
  DEFAULT_ROUTE_PREFIX,
  resolveDisplayRoutePrefix,
} from "@/lib/routePrefix";

describe("resolveDisplayRoutePrefix（展示层前缀归一化）", () => {
  it("未设置 / 空 / 纯空白回退默认 G.", () => {
    expect(resolveDisplayRoutePrefix(undefined)).toBe(DEFAULT_ROUTE_PREFIX);
    expect(resolveDisplayRoutePrefix("")).toBe(DEFAULT_ROUTE_PREFIX);
    expect(resolveDisplayRoutePrefix("   ")).toBe(DEFAULT_ROUTE_PREFIX);
    expect(DEFAULT_ROUTE_PREFIX).toBe("G.");
  });

  it("合法自定义前缀保留（含 trim）", () => {
    expect(resolveDisplayRoutePrefix("@")).toBe("@");
    expect(resolveDisplayRoutePrefix("  @ ")).toBe("@");
    expect(resolveDisplayRoutePrefix("rt.")).toBe("rt.");
  });

  it("非法值回退默认（与后端 normalize_route_prefix fail-safe 一致）", () => {
    // 结尾是字母数字（无边界符）
    expect(resolveDisplayRoutePrefix("G")).toBe(DEFAULT_ROUTE_PREFIX);
    // 含冒号
    expect(resolveDisplayRoutePrefix("g:")).toBe(DEFAULT_ROUTE_PREFIX);
    // 超长（>8）
    expect(resolveDisplayRoutePrefix("123456789")).toBe(DEFAULT_ROUTE_PREFIX);
    // 非可打印 ASCII（中文）
    expect(resolveDisplayRoutePrefix("路由。")).toBe(DEFAULT_ROUTE_PREFIX);
  });
});
