import { describe, expect, it } from "vitest";
import { getAppCapabilities, supportsAppView } from "@/config/appCapabilities";

describe("DeepSeek Harness app capabilities", () => {
  it("keeps providers app-managed and enables the shared skills/MCP surfaces", () => {
    const capabilities = getAppCapabilities("dsh");
    // fork 定制：DSH 接入统一 Skills/MCP 管理（~/.agents/skills 与 ~/.dsh/mcp.json）
    expect(capabilities.mcp).toBe(true);
    expect(capabilities.skills).toBe(true);
    expect(capabilities.defaultModel).toBe(false);
    expect(capabilities.proxy).toBe(false);
    expect(capabilities.sessions).toBe(false);
    expect(capabilities.usage).toBe(false);
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

  it("permits provider, skills, MCP, and settings views for DSH", () => {
    expect(supportsAppView("dsh", "providers")).toBe(true);
    expect(supportsAppView("dsh", "settings")).toBe(true);
    // fork 定制：DSH 接入统一 Skills/MCP 管理
    expect(supportsAppView("dsh", "mcp")).toBe(true);
    expect(supportsAppView("dsh", "skills")).toBe(true);
    expect(supportsAppView("dsh", "skillsDiscovery")).toBe(true);
    expect(supportsAppView("dsh", "sessions")).toBe(false);
    expect(supportsAppView("dsh", "universal")).toBe(false);
  });
});
