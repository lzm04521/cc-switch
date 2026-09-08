import { describe, expect, it } from "vitest";
import { validateRouteKey } from "@/components/providers/forms/RouteSettingsFields";

describe("validateRouteKey", () => {
  const existing = [
    { key: "ds", providerId: "a" },
    { key: "glm", providerId: "b" },
  ];

  it("接受合法 key", () => {
    expect(validateRouteKey("opus", existing)).toBeNull();
    expect(validateRouteKey("my-key.v2", existing)).toBeNull();
    expect(validateRouteKey("  ds3  ", existing)).toBeNull(); // trim 后合法
  });

  it("拒绝非法字符与超长", () => {
    expect(validateRouteKey("", existing)).toBe(
      "providers.form.route.keyInvalid",
    );
    expect(validateRouteKey("bad key!", existing)).toBe(
      "providers.form.route.keyInvalid",
    );
    expect(validateRouteKey("a".repeat(33), existing)).toBe(
      "providers.form.route.keyInvalid",
    );
  });

  it("拒绝保留字 default（大小写不敏感）", () => {
    expect(validateRouteKey("default", existing)).toBe(
      "providers.form.route.keyReserved",
    );
    expect(validateRouteKey("Default", existing)).toBe(
      "providers.form.route.keyReserved",
    );
  });

  it("拒绝同 app 重复 key（大小写不敏感，排除自身）", () => {
    expect(validateRouteKey("DS", existing)).toBe(
      "providers.form.route.keyDuplicate",
    );
    expect(validateRouteKey("ds", existing, "a")).toBeNull(); // 自身不算重复
  });
});
