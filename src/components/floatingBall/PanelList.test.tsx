import { render, fireEvent, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi, beforeEach } from "vitest";

const sectionsFixture = [
  {
    appType: "claude",
    currentProviderId: "p1",
    providers: [
      { id: "p1", name: "DeepSeek", icon: "deepseek", iconColor: null },
      { id: "p2", name: "Anthropic", icon: null, iconColor: null },
    ],
  },
  {
    appType: "codex",
    currentProviderId: null,
    providers: [{ id: "c1", name: "OpenAI", icon: null, iconColor: null }],
  },
];

const hidePanel = vi.fn().mockResolvedValue(true);
vi.mock("@/lib/api/floatingBall", () => ({
  floatingBallApi: {
    hidePanel: () => hidePanel(),
    onPanelBlur: vi.fn().mockResolvedValue(true),
    showMainWindow: vi.fn().mockResolvedValue(true),
  },
  useFloatingBallSections: () => ({ data: sectionsFixture }),
  FLOATING_BALL_SECTIONS_KEY: ["floating-ball-sections"],
}));

const switchFn = vi.fn().mockResolvedValue({ warnings: [] });
vi.mock("@/lib/api", () => ({
  providersApi: {
    switch: (id: string, appId: string) => switchFn(id, appId),
    onSwitched: vi.fn().mockResolvedValue(() => {}),
  },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "panel",
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
  }),
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: ({ name }: { name: string }) => <span data-name={name} />,
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

const applyProviderSwitchSideEffects = vi.fn().mockResolvedValue(undefined);
vi.mock("@/hooks/providerSwitchSideEffects", () => ({
  applyProviderSwitchSideEffects: (...args: unknown[]) =>
    applyProviderSwitchSideEffects(...args),
}));

import { PanelList } from "./PanelList";

const renderPanel = () =>
  render(
    <QueryClientProvider client={new QueryClient()}>
      <PanelList />
    </QueryClientProvider>,
  );

describe("PanelList", () => {
  beforeEach(() => {
    hidePanel.mockClear();
    switchFn.mockClear();
    switchFn.mockResolvedValue({ warnings: [] });
    applyProviderSwitchSideEffects.mockClear();
    applyProviderSwitchSideEffects.mockResolvedValue(undefined);
  });

  it("渲染 app 分组与 provider 行，当前项带 is-current 标记", () => {
    const { container } = renderPanel();
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("DeepSeek")).toBeTruthy();
    expect(screen.getByText("Anthropic")).toBeTruthy();
    expect(screen.getByText("Codex")).toBeTruthy();
    const currentRow = container.querySelector(".is-current");
    expect(currentRow).toBeTruthy();
    expect(currentRow?.textContent).toContain("DeepSeek");
  });

  it("点击 provider 行调用 switch 并收起面板", async () => {
    renderPanel();
    fireEvent.click(screen.getByText("Anthropic"));
    expect(switchFn).toHaveBeenCalledWith("p2", "claude");
    // 等待异步完成
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(applyProviderSwitchSideEffects).toHaveBeenCalledWith(
      "claude",
      "p2",
      expect.objectContaining({ switchResult: { warnings: [] } }),
    );
    expect(hidePanel).toHaveBeenCalled();
  });

  it("切换失败不收起面板", async () => {
    switchFn.mockRejectedValueOnce(new Error("switch failed"));
    renderPanel();
    fireEvent.click(screen.getByText("OpenAI"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(hidePanel).not.toHaveBeenCalled();
  });

  it("渲染打开主界面 footer 按钮", () => {
    renderPanel();
    expect(screen.getByText("打开主界面")).toBeTruthy();
  });
});
