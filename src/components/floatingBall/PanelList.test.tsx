import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi, beforeEach } from "vitest";
import type { TrayPanelState } from "@/tray-panel/api";

const stateFixture: TrayPanelState = {
  texts: {
    switchProvider: "切换供应商",
    searchPlaceholder: "搜索供应商",
    inUse: "当前",
    noMatch: "无匹配",
    openMain: "打开主界面",
    quit: "退出",
    settings: "设置",
    lightweightMode: "轻量模式",
    openPanel: "打开快捷面板",
    configTab: "配置",
    usageTab: "用量",
    resetsAt: "{when} 重置",
    resetCredits: "可重置 {count} 次",
  },
  problems: [
    { id: "fix:gemini", text: "Gemini：live 配置被外部修改，需要重新同步" },
  ],
  feedback: null,
  apps: [
    {
      app: "claude",
      name: "Claude Code",
      current: "BigModel",
      mode: "聚合",
      modeKind: "stack",
      header: null,
      attention: null,
      quota: null,
      addId: null,
      addText: "添加供应商",
      rows: [
        {
          kind: "provider",
          id: "prov:p1",
          text: "BigModel",
          enabled: true,
          checked: true,
          provider: {
            name: "BigModel",
            hint: "聚合 · 已用 32%",
            icon: null,
            iconColor: null,
            quota: null,
          },
        },
        {
          kind: "provider",
          id: "prov:p2",
          text: "DeepSeek 官方",
          enabled: true,
          checked: false,
          provider: {
            name: "DeepSeek 官方",
            hint: "余额 ¥96.20",
            icon: null,
            iconColor: null,
            quota: null,
          },
        },
      ],
    },
  ],
  lightweight: false,
  quitHint: null,
};

vi.mock("@/tray-panel/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/tray-panel/api")>()),
  trayPanelApi: {
    state: vi.fn(),
    action: vi.fn().mockResolvedValue(undefined),
    placement: vi.fn(),
    layout: vi.fn(),
    hide: vi.fn(),
  },
}));

vi.mock("@/tray-panel/UsageTab", () => ({
  UsageTab: ({ visible }: { visible: boolean }) => (
    <div data-testid="usage-tab" data-visible={String(visible)} />
  ),
}));

const hidePanel = vi.fn().mockResolvedValue(true);
const setSubPanelVisible = vi.fn().mockResolvedValue(true);
vi.mock("@/lib/api/floatingBall", () => ({
  floatingBallApi: {
    hidePanel: () => hidePanel(),
    onPanelBlur: vi.fn().mockResolvedValue(true),
    showMainWindow: vi.fn().mockResolvedValue(true),
    // 测试走 overlay（覆盖层）路径：split 双栏依赖后端画布布局与窗口裁剪
    getPanelLayout: vi.fn().mockResolvedValue({
      layout: "overlay",
      side: "right",
      mainWidth: 300,
      subWidth: 300,
      gap: 8,
    }),
    setSubPanelVisible: (visible: boolean) => setSubPanelVisible(visible),
  },
}));

vi.mock("@/lib/api", () => ({
  providersApi: {
    onSwitched: vi.fn().mockResolvedValue(() => {}),
  },
}));

vi.mock("@/lib/query/usage", () => ({
  useUsageSummary: () => ({ data: null }),
  usageKeys: { all: ["usage"] },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: "panel",
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
  }),
}));

import { trayPanelApi } from "@/tray-panel/api";
import { PanelList } from "./PanelList";

const renderPanel = () =>
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <PanelList />
    </QueryClientProvider>,
  );

async function renderReady() {
  vi.mocked(trayPanelApi.state).mockResolvedValue(stateFixture);
  const view = renderPanel();
  // 等待 query 完成渲染出应用行
  await screen.findByRole("button", { name: "Claude Code BigModel · 聚合" });
  return view;
}

describe("PanelList（悬浮球面板 = 托盘面板内容 + 底部保留区）", () => {
  beforeEach(() => {
    vi.mocked(trayPanelApi.state).mockReset();
    vi.mocked(trayPanelApi.action).mockClear();
    hidePanel.mockClear();
    // 页签记忆写 localStorage（jsdom 跨用例共享），逐用例复位到配置页
    window.localStorage.removeItem("cc-switch-tray-tab");
  });

  it("渲染头部、配置/用量页签、问题提醒、应用行与轻量模式", async () => {
    await renderReady();
    expect(screen.getByText("CC Switch")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "配置" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "用量" })).toBeTruthy();
    expect(
      screen.getByText("Gemini：live 配置被外部修改，需要重新同步"),
    ).toBeTruthy();
    // 轻量模式开关（footer 只保留它；退出按钮不提供）
    expect(screen.getByRole("switch", { name: "轻量模式" })).toBeTruthy();
    expect(screen.queryByText("退出")).toBeNull();
  });

  it("底部保留区渲染今日统计与打开主界面按钮", async () => {
    await renderReady();
    expect(screen.getByText("今日总Token")).toBeTruthy();
    const openMain = screen.getByRole("button", { name: "打开主界面" });
    expect(openMain).toBeTruthy();
  });

  it("点击应用行展开覆盖层子面板，点供应商行走 tray_panel_action", async () => {
    await renderReady();
    fireEvent.click(
      screen.getByRole("button", { name: "Claude Code BigModel · 聚合" }),
    );
    const overlay = screen.getByRole("dialog", { name: "Claude Code" });
    expect(overlay).toBeTruthy();
    expect(screen.getByText("切换供应商")).toBeTruthy();
    expect(screen.getByText("DeepSeek 官方")).toBeTruthy();

    fireEvent.click(screen.getByText("DeepSeek 官方"));
    expect(trayPanelApi.action).toHaveBeenCalledWith("prov:p2");
  });

  it("Esc 先收覆盖层、再收面板", async () => {
    await renderReady();
    fireEvent.click(
      screen.getByRole("button", { name: "Claude Code BigModel · 聚合" }),
    );
    expect(screen.getByRole("dialog", { name: "Claude Code" })).toBeTruthy();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Claude Code" })).toBeNull();
    expect(hidePanel).not.toHaveBeenCalled();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(hidePanel).toHaveBeenCalled();
  });

  it("切到用量页签渲染 UsageTab（visible=true）", async () => {
    await renderReady();
    fireEvent.click(screen.getByRole("tab", { name: "用量" }));
    const usage = screen.getByTestId("usage-tab");
    expect(usage.dataset.visible).toBe("true");
  });

  it("问题行与轻量模式点击走 tray_panel_action", async () => {
    await renderReady();
    fireEvent.click(
      screen.getByText("Gemini：live 配置被外部修改，需要重新同步"),
    );
    expect(trayPanelApi.action).toHaveBeenCalledWith("fix:gemini");

    fireEvent.click(screen.getByRole("switch", { name: "轻量模式" }));
    expect(trayPanelApi.action).toHaveBeenCalledWith("lightweight_mode");
  });
});
