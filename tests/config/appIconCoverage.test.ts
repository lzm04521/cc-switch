import { describe, expect, it } from "vitest";
import { getIcon, hasIcon } from "@/icons/extracted";
// AppGlyph 用 showFallback={false} 渲染应用图标：注册表缺条目时该应用的图标位是空白
// （workbuddy 曾因此无图标）。这里锁死「每个应用的图标名都能解析」。
import { APP_ICON_NAME } from "@/components/shell/AppGlyph";

describe("AppGlyph icon coverage", () => {
  it.each(Object.entries(APP_ICON_NAME))(
    "app %s resolves to a registered icon",
    (_app, icon) => {
      expect(hasIcon(icon)).toBe(true);
    },
  );

  it("workbuddy icon is the W monogram", () => {
    expect(getIcon("workbuddy")).toContain("<title>WorkBuddy</title>");
  });
});
