import { describe, expect, it, vi, beforeEach } from "vitest";
import type { TFunction } from "i18next";

const getAll = vi.fn();
const settingsGet = vi.fn();
const applyClaudePluginConfig = vi.fn();
const getProxyStatus = vi.fn();
const getProxyTakeoverStatus = vi.fn();

vi.mock("@/lib/api", () => ({
  providersApi: {
    getAll: (appId: string) => getAll(appId),
  },
  settingsApi: {
    get: () => settingsGet(),
    applyClaudePluginConfig: (opts: { official: boolean }) =>
      applyClaudePluginConfig(opts),
  },
}));

vi.mock("@/lib/api/proxy", () => ({
  proxyApi: {
    getProxyStatus: () => getProxyStatus(),
    getProxyTakeoverStatus: () => getProxyTakeoverStatus(),
  },
}));

const toastWarning = vi.fn();
const toastError = vi.fn();
const toastSuccess = vi.fn();
vi.mock("sonner", () => ({
  toast: {
    warning: (...args: unknown[]) => toastWarning(...args),
    error: (...args: unknown[]) => toastError(...args),
    success: (...args: unknown[]) => toastSuccess(...args),
  },
}));

import { applyProviderSwitchSideEffects } from "./providerSwitchSideEffects";

// 简单 t 桩：返回 defaultValue，便于断言文案
const t = ((key: string, opts?: { defaultValue?: string }) =>
  opts?.defaultValue ?? key) as TFunction;

const runningProxy = () => ({
  running: true,
  address: "",
  port: 0,
  active_connections: 0,
  total_requests: 0,
  success_requests: 0,
  failed_requests: 0,
  success_rate: 0,
  uptime_seconds: 0,
  current_provider: null,
  current_provider_id: null,
  last_request_at: null,
  last_error: null,
  failover_count: 0,
});
const stoppedProxy = () => ({ ...runningProxy(), running: false });
const takeoverNone = () => ({
  claude: false,
  codex: false,
  gemini: false,
  grokbuild: false,
  opencode: false,
  openclaw: false,
  hermes: false,
});

describe("applyProviderSwitchSideEffects", () => {
  beforeEach(() => {
    getAll.mockReset();
    settingsGet.mockReset();
    applyClaudePluginConfig.mockReset();
    getProxyStatus.mockReset();
    getProxyTakeoverStatus.mockReset();
    toastWarning.mockClear();
    toastError.mockClear();
    toastSuccess.mockClear();
  });

  it("Claude 插件联动：官方 provider 切换后写入 official: true", async () => {
    getAll.mockResolvedValue({
      p1: {
        id: "p1",
        name: "Claude Official",
        category: "official",
        settingsConfig: {},
        meta: {},
      },
    });
    settingsGet.mockResolvedValue({ enableClaudePluginIntegration: true });
    applyClaudePluginConfig.mockResolvedValue(true);
    getProxyStatus.mockResolvedValue(stoppedProxy());
    getProxyTakeoverStatus.mockResolvedValue(takeoverNone());

    await applyProviderSwitchSideEffects("claude", "p1", { t });

    expect(applyClaudePluginConfig).toHaveBeenCalledWith({ official: true });
  });

  it("代理必需警告：需本地路由且代理未就绪时弹 warning", async () => {
    getAll.mockResolvedValue({
      p1: {
        id: "p1",
        name: "OpenAI Chat",
        category: "custom",
        settingsConfig: {},
        meta: { apiFormat: "openai_chat" },
      },
    });
    settingsGet.mockResolvedValue({});
    getProxyStatus.mockResolvedValue(stoppedProxy());
    getProxyTakeoverStatus.mockResolvedValue(takeoverNone());

    await applyProviderSwitchSideEffects("claude", "p1", { t });

    expect(toastWarning).toHaveBeenCalled();
    expect(toastWarning.mock.calls[0][0]).toContain("代理");
  });
});
