import { describe, expect, it } from "vitest";
import { getAppCapabilities, supportsAppView } from "@/config/appCapabilities";

describe("DeepSeek Harness app capabilities", () => {
  it("fully manages DSH with skills/MCP/sessions/usage and no proxy", () => {
    const capabilities = getAppCapabilities("dsh");
    // 上游 v4.0.22 全量接管：provider（additive + 账号登录）、skills、MCP、
    // 会话与导入用量；不走本地代理。fork：托盘/悬浮球行维持关闭。
    expect(capabilities.mcp).toBe(true);
    expect(capabilities.skills).toBe(true);
    expect(capabilities.sessions).toBe(true);
    expect(capabilities.usage).toBe(true);
    expect(capabilities.defaultModel).toBe(false);
    expect(capabilities.proxy).toBe(false);
    expect(capabilities.prompts).toBe(false);
    expect(capabilities.tray).toBe(false);
  });

  it("keeps Claude Desktop shared Claude surfaces available", () => {
    const capabilities = getAppCapabilities("claude-desktop");

    expect(capabilities).toMatchObject({
      profiles: true,
      prompts: true,
      skills: true,
      mcp: true,
      sessions: true,
      failover: false,
      universal: false,
      usage: true,
    });
    expect(supportsAppView("claude-desktop", "prompts")).toBe(true);
    expect(supportsAppView("claude-desktop", "skills")).toBe(true);
    expect(supportsAppView("claude-desktop", "mcp")).toBe(true);
    expect(supportsAppView("claude-desktop", "sessions")).toBe(true);
    expect(supportsAppView("claude-desktop", "universal")).toBe(false);
  });

  it("permits provider, skills, MCP, sessions, and settings views for DSH", () => {
    expect(supportsAppView("dsh", "providers")).toBe(true);
    expect(supportsAppView("dsh", "settings")).toBe(true);
    expect(supportsAppView("dsh", "mcp")).toBe(true);
    expect(supportsAppView("dsh", "skills")).toBe(true);
    expect(supportsAppView("dsh", "skillsDiscovery")).toBe(true);
    expect(supportsAppView("dsh", "sessions")).toBe(true);
    expect(supportsAppView("dsh", "universal")).toBe(false);
  });

  it("keeps WorkBuddy providers app-managed with only skills/MCP surfaces", () => {
    const capabilities = getAppCapabilities("workbuddy");
    // fork 定制：WorkBuddy 仅接入统一 Skills/MCP 管理，provider 由应用内自管
    expect(capabilities.mcp).toBe(true);
    expect(capabilities.skills).toBe(true);
    expect(capabilities.prompts).toBe(false);
    expect(capabilities.proxy).toBe(false);
    expect(capabilities.failover).toBe(false);
    expect(capabilities.sessions).toBe(false);
    expect(capabilities.universal).toBe(false);
    expect(capabilities.usage).toBe(false);
    expect(capabilities.tray).toBe(false);
    expect(capabilities.defaultModel).toBe(false);
    expect(supportsAppView("workbuddy", "providers")).toBe(true);
    expect(supportsAppView("workbuddy", "settings")).toBe(true);
    expect(supportsAppView("workbuddy", "skills")).toBe(true);
    expect(supportsAppView("workbuddy", "skillsDiscovery")).toBe(true);
    expect(supportsAppView("workbuddy", "mcp")).toBe(true);
    expect(supportsAppView("workbuddy", "sessions")).toBe(false);
    expect(supportsAppView("workbuddy", "prompts")).toBe(false);
  });
});
