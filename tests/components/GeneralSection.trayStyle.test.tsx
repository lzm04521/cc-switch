import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { GeneralSection } from "@/components/settings/sections/GeneralSection";
import type { SettingsFormState } from "@/hooks/useSettings";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/components/theme-provider", () => ({
  useTheme: () => ({ theme: "system", setTheme: vi.fn() }),
}));

vi.mock("@/components/settings/TerminalSettings", () => ({
  TerminalSelect: () => null,
}));

const platform = vi.hoisted(() => ({ linux: false }));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isLinux: () => platform.linux,
}));

const baseSettings = {
  showInTray: true,
  minimizeToTrayOnClose: true,
  language: "zh",
} as SettingsFormState;

function renderSection(settings: Partial<SettingsFormState> = {}) {
  const onAutoSave = vi.fn().mockResolvedValue(true);
  render(
    <GeneralSection
      settings={{ ...baseSettings, ...settings }}
      onAutoSave={onAutoSave}
      onOpenApps={vi.fn()}
    />,
  );
  return onAutoSave;
}

describe("GeneralSection tray style", () => {
  beforeEach(() => {
    platform.linux = false;
  });

  it("defaults to the panel and saves the native menu when Menu is picked", () => {
    const onAutoSave = renderSection();
    const group = screen.getByRole("group", {
      name: "settings.general.trayStyle",
    });
    expect(group).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "settings.general.trayStylePanel" }),
    ).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(
      screen.getByRole("button", { name: "settings.general.trayStyleMenu" }),
    );
    expect(onAutoSave).toHaveBeenCalledWith({ nativeTrayMenu: true });
  });

  it("switches back to the panel", () => {
    const onAutoSave = renderSection({ nativeTrayMenu: true });
    fireEvent.click(
      screen.getByRole("button", { name: "settings.general.trayStylePanel" }),
    );
    expect(onAutoSave).toHaveBeenCalledWith({ nativeTrayMenu: false });
  });

  it("is hidden on Linux, which only has the native menu", () => {
    platform.linux = true;
    renderSection();
    expect(
      screen.queryByRole("group", { name: "settings.general.trayStyle" }),
    ).not.toBeInTheDocument();
  });
});

describe("GeneralSection floating ball switch", () => {
  beforeEach(() => {
    platform.linux = false;
  });

  it("renders above the tray style row and defaults to on", () => {
    const onAutoSave = renderSection();
    const ballSwitch = screen.getByRole("switch", {
      name: "settings.floatingBall",
    });
    expect(ballSwitch).toHaveAttribute("aria-checked", "true");
    // 开关在「托盘样式」行上方（同属 !isLinux() 分支）
    const styleGroup = screen.getByRole("group", {
      name: "settings.general.trayStyle",
    });
    expect(
      Boolean(ballSwitch.compareDocumentPosition(styleGroup) &
        Node.DOCUMENT_POSITION_FOLLOWING),
    ).toBe(true);
    fireEvent.click(ballSwitch);
    expect(onAutoSave).toHaveBeenCalledWith({
      floatingBall: { enabled: false },
    });
  });

  it("keeps the panel size settings when toggling", () => {
    const onAutoSave = renderSection({
      floatingBall: { enabled: true, panelWidth: 360, panelHeight: 560 },
    });
    fireEvent.click(
      screen.getByRole("switch", { name: "settings.floatingBall" }),
    );
    expect(onAutoSave).toHaveBeenCalledWith({
      floatingBall: { enabled: false, panelWidth: 360, panelHeight: 560 },
    });
  });

  it("is hidden on Linux together with the tray style row", () => {
    platform.linux = true;
    renderSection();
    expect(
      screen.queryByRole("switch", { name: "settings.floatingBall" }),
    ).not.toBeInTheDocument();
  });
});
