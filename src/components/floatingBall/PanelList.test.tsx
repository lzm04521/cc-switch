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
// 可变注入：各用例按需填充用量缓存快照（undefined = 无缓存）
let usageSnapshots: Array<{
  appType: string;
  providerId: string;
  result: {
    success: boolean;
    data?: Array<{
      planName?: string;
      remaining?: number;
      total?: number;
      used?: number;
      unit?: string;
      extra?: string;
    }>;
    error?: string;
  };
  queriedAt: number;
}> | undefined;
vi.mock("@/lib/api/floatingBall", () => ({
  floatingBallApi: {
    hidePanel: () => hidePanel(),
    onPanelBlur: vi.fn().mockResolvedValue(true),
    showMainWindow: vi.fn().mockResolvedValue(true),
  },
  useFloatingBallSections: () => ({ data: sectionsFixture }),
  useProviderUsageCache: () => ({ data: usageSnapshots }),
  FLOATING_BALL_SECTIONS_KEY: ["floating-ball-sections"],
  PROVIDER_USAGE_CACHE_KEY: ["provider-usage-cache"],
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
    usageSnapshots = undefined;
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

  it("无用量缓存时行下不渲染用量子行", () => {
    const { container } = renderPanel();
    expect(container.querySelector(".floating-ball-row-usage")).toBeNull();
    expect(container.querySelector(".has-usage")).toBeNull();
  });

  it("缓存命中时行下渲染余额子行（剩余值 + 单位 + 相对时间）", () => {
    usageSnapshots = [
      {
        appType: "claude",
        providerId: "p1",
        result: {
          success: true,
          data: [
            {
              planName: "标准套餐",
              remaining: 9.85,
              total: 20,
              used: 10.15,
              unit: "$",
            },
          ],
        },
        queriedAt: Date.now() - 5 * 60 * 1000,
      },
    ];
    const { container } = renderPanel();
    const subRow = container.querySelector(".floating-ball-row-usage");
    expect(subRow).toBeTruthy();
    expect(subRow?.textContent).toContain("9.85");
    expect(subRow?.textContent).toContain("$");
    // 行卡片带 has-usage（wrap 布局开关）
    expect(container.querySelector(".has-usage")).toBeTruthy();
    // 未命中的 provider（p2/c1）不渲染子行
    const subRows = container.querySelectorAll(".floating-ball-row-usage");
    expect(subRows.length).toBe(1);
  });

  it("Token Plan 窗口按百分比渲染（多窗口全部显示，带重置倒计时）", () => {
    usageSnapshots = [
      {
        appType: "claude",
        providerId: "p1",
        result: {
          success: true,
          data: [
            {
              planName: "5h",
              remaining: 22,
              total: 100,
              used: 78,
              unit: "%",
              extra: JSON.stringify({
                resetsAt: new Date(Date.now() + 4 * 3600 * 1000).toISOString(),
              }),
            },
            { planName: "7d", remaining: 64, total: 100, used: 36, unit: "%" },
          ],
        },
        queriedAt: Date.now() - 60 * 1000,
      },
    ];
    const { container } = renderPanel();
    const subRow = container.querySelector(".floating-ball-row-usage");
    expect(subRow?.textContent).toContain("5h");
    expect(subRow?.textContent).toContain("7d");
    // resetsAt 解析为倒计时纯文本（如 "3h59m"；floor 取整，不锚定具体数值）
    expect(subRow?.textContent).toMatch(/\d+h\d*m/);
  });

  it("失败缓存显示错误信息（不吞错）", () => {
    usageSnapshots = [
      {
        appType: "codex",
        providerId: "c1",
        result: { success: false, data: undefined, error: "API key is empty" },
        queriedAt: Date.now() - 10 * 60 * 1000,
      },
    ];
    const { container } = renderPanel();
    const subRow = container.querySelector(".floating-ball-row-usage");
    expect(subRow?.textContent).toContain("API key is empty");
    expect(subRow?.className).toContain("floating-ball-row-usage");
  });

  it("成功但无数据时不渲染子行（与主界面 UsageFooter 一致）", () => {
    usageSnapshots = [
      {
        appType: "claude",
        providerId: "p1",
        result: { success: true, data: [], queriedAt: Date.now() },
      } as never,
    ];
    const { container } = renderPanel();
    expect(container.querySelector(".floating-ball-row-usage")).toBeNull();
  });
});
