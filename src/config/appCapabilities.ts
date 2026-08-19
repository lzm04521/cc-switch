import type { AppId } from "@/lib/api";

/** UI and integration capabilities exposed for one application. */
export interface AppCapabilities {
  proxy: boolean;
  failover: boolean;
  profiles: boolean;
  prompts: boolean;
  skills: boolean;
  mcp: boolean;
  sessions: boolean;
  universal: boolean;
  terminal: boolean;
  tray: boolean;
  usage: boolean;
  agents: boolean;
  workspace: boolean;
  openclawTools: boolean;
  hermesMemory: boolean;
  defaultModel: boolean;
}

const genericDefaults: AppCapabilities = {
  proxy: true,
  failover: true,
  profiles: false,
  prompts: true,
  skills: true,
  mcp: true,
  sessions: true,
  universal: true,
  terminal: false,
  tray: true,
  usage: true,
  agents: false,
  workspace: false,
  openclawTools: false,
  hermesMemory: false,
  defaultModel: false,
};

const generic = (
  overrides: Partial<AppCapabilities> = {},
): AppCapabilities => ({
  ...genericDefaults,
  ...overrides,
});

/** Application capability table used by the shell and navigation guards. */
export const APP_CAPABILITIES: Record<AppId, AppCapabilities> = {
  claude: generic({ profiles: true, terminal: true }),
  "claude-desktop": generic({
    failover: false,
    // Claude Desktop shares the Claude Code prompt, Skills, MCP, and session
    // surfaces.  Its provider and profile storage remain app-specific.
    profiles: true,
    prompts: true,
    skills: true,
    mcp: true,
    sessions: true,
    universal: false,
    usage: true,
  }),
  codex: generic({ profiles: true }),
  gemini: generic(),
  grokbuild: generic(),
  opencode: generic({
    proxy: false,
    failover: false,
    universal: false,
  }),
  openclaw: generic({
    proxy: false,
    failover: false,
    prompts: false,
    skills: false,
    mcp: false,
    universal: false,
    agents: true,
    workspace: true,
    openclawTools: true,
  }),
  hermes: generic({
    proxy: false,
    failover: false,
    prompts: false,
    hermesMemory: true,
  }),
  pi: generic({
    // Pi manages its own MCP surface separately; it keeps the shared
    // prompts, skills, sessions, and usage integrations.
    mcp: false,
    proxy: false,
    failover: false,
  }),
  // ZCode providers are managed in-app; it keeps the shared skills, prompts,
  // MCP, and sessions surfaces without the proxy/universal integrations.
  zcode: generic({
    proxy: false,
    failover: false,
    universal: false,
  }),
  // DSH providers are managed in-app (settings.yaml/.credentials.yaml);
  // cc-switch only manages its skills deployment and its profile patch
  // MCP config (cordis.patch.yml).
  dsh: generic({
    proxy: false,
    failover: false,
    prompts: false,
    skills: true,
    mcp: true,
    sessions: false,
    universal: false,
    tray: false,
    usage: false,
    defaultModel: false,
  }),
};

/** Return the immutable capability row for an application. */
export function getAppCapabilities(appId: AppId): AppCapabilities {
  return APP_CAPABILITIES[appId];
}

/** Return whether a saved/main-window view is valid for the selected app. */
export function supportsAppView(appId: AppId, view: string): boolean {
  const capabilities = getAppCapabilities(appId);
  switch (view) {
    case "providers":
    case "settings":
      return true;
    case "prompts":
      return capabilities.prompts;
    case "skills":
    case "skillsDiscovery":
      return capabilities.skills;
    case "mcp":
      return capabilities.mcp;
    case "sessions":
      return capabilities.sessions;
    case "universal":
      return capabilities.universal;
    case "agents":
      return capabilities.agents;
    case "workspace":
      return capabilities.workspace;
    case "openclawEnv":
    case "openclawTools":
    case "openclawAgents":
      return capabilities.openclawTools;
    case "hermesMemory":
      return capabilities.hermesMemory;
    default:
      return false;
  }
}
