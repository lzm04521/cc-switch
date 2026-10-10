import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * 关于页「更新日志」按钮的链接回归：fork 发版号是 <上游版本>-<N>（如 4.0.6-1），
 * 只有本 fork 仓库存在对应 tag，上游仓库没有；上游 merge 时该 URL 极易被静默改回，
 * 这里钉死指向本 fork，防止合并无声回归。
 */
const mocks = vi.hoisted(() => ({
  openExternal: vi.fn(),
  getVersion: vi.fn(),
  update: {
    hasUpdate: false,
    updateInfo: undefined as { availableVersion?: string } | undefined,
    checkUpdate: vi.fn(),
    resetDismiss: vi.fn(),
    isChecking: false,
  },
}));

vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/lib/toast", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => mocks.update,
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh" } }),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: () => mocks.getVersion(),
}));

import { AboutSection } from "@/components/settings/AboutSection";

const FORK_RELEASES = "https://github.com/lzm04521/cc-switch/releases";

async function clickReleaseNotes() {
  render(<AboutSection isPortable={false} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "settings.releaseNotes" }),
  );
}

describe("AboutSection 更新日志链接", () => {
  beforeEach(() => {
    mocks.openExternal.mockReset().mockResolvedValue(undefined);
    mocks.getVersion.mockReset().mockResolvedValue("4.0.6-1");
    mocks.update = {
      hasUpdate: false,
      updateInfo: undefined,
      checkUpdate: vi.fn(),
      resetDismiss: vi.fn(),
      isChecking: false,
    };
  });

  it("打开本 fork 仓库对应版本的 tag 页", async () => {
    await clickReleaseNotes();
    await waitFor(() =>
      expect(mocks.openExternal).toHaveBeenCalledWith(
        `${FORK_RELEASES}/tag/v4.0.6-1`,
      ),
    );
  });

  it("版本未知时退到本 fork 仓库 Releases 列表", async () => {
    mocks.getVersion.mockRejectedValue(new Error("no version"));
    await clickReleaseNotes();
    await waitFor(() =>
      expect(mocks.openExternal).toHaveBeenCalledWith(FORK_RELEASES),
    );
  });
});
