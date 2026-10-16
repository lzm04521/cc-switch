import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ComponentProps, PropsWithChildren } from "react";
import { useForm } from "react-hook-form";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ClaudeFormFields } from "@/components/providers/forms/ClaudeFormFields";
import { createClaudeStackModelRow } from "@/components/providers/forms/ClaudeStackModelsField";
import { Form } from "@/components/ui/form";

const copilotApiMock = vi.hoisted(() => ({
  copilotGetModels: vi.fn(),
  copilotGetModelsForAccount: vi.fn(),
}));

const modelFetchApiMock = vi.hoisted(() => ({
  fetchCodexOauthModels: vi.fn(),
  fetchModelsForConfig: vi.fn(),
  showFetchModelsError: vi.fn(),
}));

vi.mock("@/lib/api/copilot", () => ({
  copilotGetModels: copilotApiMock.copilotGetModels,
  copilotGetModelsForAccount: copilotApiMock.copilotGetModelsForAccount,
}));

vi.mock("@/lib/api/model-fetch", () => ({
  fetchCodexOauthModels: modelFetchApiMock.fetchCodexOauthModels,
  fetchModelsForConfig: modelFetchApiMock.fetchModelsForConfig,
  showFetchModelsError: modelFetchApiMock.showFetchModelsError,
}));

vi.mock("@/components/providers/forms/CopilotAuthSection", () => ({
  CopilotAuthSection: () => <div data-testid="copilot-auth-section" />,
}));

vi.mock("@/components/providers/forms/CodexOAuthSection", () => ({
  CodexOAuthSection: () => <div data-testid="codex-oauth-section" />,
}));

type ClaudeFormFieldsProps = ComponentProps<typeof ClaudeFormFields>;

const FormShell = ({ children }: PropsWithChildren) => {
  const form = useForm();

  return <Form {...form}>{children}</Form>;
};

const renderCopilotForm = (overrides: Partial<ClaudeFormFieldsProps> = {}) => {
  const props: ClaudeFormFieldsProps = {
    shouldShowApiKey: false,
    apiKey: "",
    onApiKeyChange: vi.fn(),
    category: "official",
    shouldShowApiKeyLink: false,
    websiteUrl: "",
    isCopilotPreset: true,
    usesOAuth: true,
    isCopilotAuthenticated: true,
    selectedGitHubAccountId: "gh-1",
    onGitHubAccountSelect: vi.fn(),
    isCodexOauthPreset: false,
    isCodexOauthAuthenticated: false,
    selectedCodexAccountId: null,
    onCodexAccountSelect: vi.fn(),
    codexFastMode: false,
    onCodexFastModeChange: vi.fn(),
    templateValueEntries: [],
    templateValues: {},
    templatePresetName: "",
    onTemplateValueChange: vi.fn(),
    shouldShowSpeedTest: false,
    baseUrl: "",
    onBaseUrlChange: vi.fn(),
    isEndpointModalOpen: false,
    onEndpointModalToggle: vi.fn(),
    onCustomEndpointsChange: vi.fn(),
    autoSelect: false,
    onAutoSelectChange: vi.fn(),
    showEndpointTools: true,
    shouldShowModelSelector: true,
    claudeModel: "",
    defaultHaikuModel: "",
    defaultHaikuModelName: "",
    defaultSonnetModel: "claude-sonnet",
    defaultSonnetModelName: "Claude Sonnet",
    defaultOpusModel: "",
    defaultOpusModelName: "",
    defaultFableModel: "",
    defaultFableModelName: "",
    subagentModel: "",
    onModelChange: vi.fn(),
    speedTestEndpoints: [],
    apiFormat: "anthropic",
    onApiFormatChange: vi.fn(),
    apiKeyField: "ANTHROPIC_AUTH_TOKEN",
    onApiKeyFieldChange: vi.fn(),
    isFullUrl: false,
    onFullUrlChange: vi.fn(),
    customUserAgent: "",
    onCustomUserAgentChange: vi.fn(),
    localProxyHeadersOverride: "",
    onLocalProxyHeadersOverrideChange: vi.fn(),
    localProxyBodyOverride: "",
    onLocalProxyBodyOverrideChange: vi.fn(),
    localProxyBodyRewrites: "",
    onLocalProxyBodyRewritesChange: vi.fn(),
    ...overrides,
  };

  return render(
    <FormShell>
      <ClaudeFormFields {...props} />
    </FormShell>,
  );
};

const renderCodexOauthForm = (overrides: Partial<ClaudeFormFieldsProps> = {}) =>
  renderCopilotForm({
    isCopilotPreset: false,
    isCopilotAuthenticated: false,
    selectedGitHubAccountId: null,
    isCodexOauthPreset: true,
    isCodexOauthAuthenticated: true,
    selectedCodexAccountId: "chatgpt-1",
    ...overrides,
  });

describe("ClaudeFormFields", () => {
  beforeEach(() => {
    copilotApiMock.copilotGetModels.mockResolvedValue([]);
    copilotApiMock.copilotGetModelsForAccount.mockResolvedValue([]);
    modelFetchApiMock.fetchCodexOauthModels.mockResolvedValue([]);
    modelFetchApiMock.fetchModelsForConfig.mockResolvedValue([]);
  });

  it("不会在 Copilot 表单打开时自动获取模型列表", () => {
    renderCopilotForm();

    expect(copilotApiMock.copilotGetModels).not.toHaveBeenCalled();
    expect(copilotApiMock.copilotGetModelsForAccount).not.toHaveBeenCalled();
  });

  it("点击获取模型列表后才请求当前 Copilot 账号的模型", async () => {
    renderCopilotForm();

    fireEvent.click(
      screen.getByRole("button", {
        name: "providerForm.fetchModels",
      }),
    );

    await waitFor(() => {
      expect(copilotApiMock.copilotGetModelsForAccount).toHaveBeenCalledWith(
        "gh-1",
      );
    });
    expect(copilotApiMock.copilotGetModels).not.toHaveBeenCalled();
  });

  it("不会在 Codex OAuth 表单打开时自动获取模型列表", () => {
    renderCodexOauthForm();

    expect(modelFetchApiMock.fetchCodexOauthModels).not.toHaveBeenCalled();
  });

  it("点击获取模型列表后才请求当前 Codex OAuth 账号的模型", async () => {
    renderCodexOauthForm();

    fireEvent.click(
      screen.getByRole("button", {
        name: "providerForm.fetchModels",
      }),
    );

    await waitFor(() => {
      expect(modelFetchApiMock.fetchCodexOauthModels).toHaveBeenCalledWith(
        "chatgpt-1",
      );
    });
  });

  it("一键设置会同时写入 Subagent 模型", () => {
    const onModelChange = vi.fn();
    renderCopilotForm({
      claudeModel: "shared-model[1M]",
      defaultSonnetModel: "",
      defaultSonnetModelName: "",
      onModelChange,
    });

    fireEvent.click(
      screen.getByRole("button", {
        name: "一键设置",
      }),
    );

    expect(onModelChange).toHaveBeenCalledWith(
      "CLAUDE_CODE_SUBAGENT_MODEL",
      "shared-model[1M]",
    );
  });

  it("Haiku 行也能勾 1M：勾选状态即模型名的 [1M] 后缀", () => {
    const onModelChange = vi.fn();
    renderCopilotForm({
      isCopilotPreset: false,
      usesOAuth: false,
      defaultHaikuModel: "glm-5.3-flash[1M]",
      defaultHaikuModelName: "",
      onModelChange,
    });

    const haikuRow = screen.getByText("Haiku").parentElement!;
    expect(within(haikuRow).getByDisplayValue("glm-5.3-flash")).toBeTruthy();
    const checkbox = within(haikuRow).getByRole("checkbox");
    expect(checkbox).toBeChecked();

    fireEvent.click(checkbox);
    expect(onModelChange).toHaveBeenCalledWith(
      "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "glm-5.3-flash",
    );
  });
});

describe("ClaudeFormFields stack 变体：聚合默认分组模型映射区块", () => {
  it("门控开启且未配过映射（values 缺省）也渲染区块——首次配置入口", () => {
    // ProviderForm 侧 meta.stackRoleMapping 不存在时传 undefined；不回退的话
    // 区块永不渲染，用户没有首次配置的入口（曾导致 v4.0.12-4 编辑页看不到映射设置）。
    renderCopilotForm({
      variant: "stack",
      isCopilotPreset: false,
      isCopilotAuthenticated: false,
      stackModelRows: [
        createClaudeStackModelRow({ model: "glm-5.2" }),
        createClaudeStackModelRow({ model: "glm-flash" }),
      ],
      stackRoleMappingEnabled: true,
      onStackRoleMappingChange: vi.fn(),
    });
    expect(screen.getByText("模型映射")).toBeInTheDocument();
    expect(screen.getByText("Sonnet")).toBeInTheDocument();
    expect(screen.getByText("Haiku")).toBeInTheDocument();
  });

  it("门控开启时渲染四档映射区块，未开启不渲染", () => {
    // 不传 onStackModelRowsChange：模型列表区块跳过（避免 settings query 依赖），映射区块独立渲染。
    const { unmount } = renderCopilotForm({
      variant: "stack",
      isCopilotPreset: false,
      isCopilotAuthenticated: false,
      stackModelRows: [
        createClaudeStackModelRow({ model: "glm-5.2" }),
        createClaudeStackModelRow({ model: "glm-flash" }),
      ],
      stackRoleMappingEnabled: true,
      stackRoleMappingValues: {
        sonnet: "",
        opus: "",
        fable: "",
        haiku: "glm-flash",
      },
      onStackRoleMappingChange: vi.fn(),
    });
    expect(screen.getByText("模型映射")).toBeInTheDocument();
    expect(screen.getByText("Haiku")).toBeInTheDocument();
    unmount();

    renderCopilotForm({
      variant: "stack",
      isCopilotPreset: false,
      isCopilotAuthenticated: false,
      stackModelRows: [createClaudeStackModelRow({ model: "glm-5.2" })],
      stackRoleMappingEnabled: false,
      stackRoleMappingValues: { sonnet: "", opus: "", fable: "", haiku: "" },
      onStackRoleMappingChange: vi.fn(),
    });
    expect(screen.queryByText("模型映射")).not.toBeInTheDocument();
  });
});
