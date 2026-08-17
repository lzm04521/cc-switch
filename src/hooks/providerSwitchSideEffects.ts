import type { TFunction } from "i18next";
import { toast } from "sonner";
import { providersApi, settingsApi, type AppId } from "@/lib/api";
import type { SwitchResult } from "@/lib/api/providers";
import { proxyApi } from "@/lib/api/proxy";
import type { ProxyTakeoverStatus } from "@/types/proxy";
import type { Provider } from "@/types";
import { extractErrorMessage } from "@/utils/errorUtils";
import { isOAuthProviderType } from "@/config/constants";
import { isProxyAppId } from "@/config/appConfig";
import {
  extractCodexWireApi,
  isCodexAnthropicWireApi,
  isCodexChatWireApi,
} from "@/utils/providerConfigUtils";
import { providerNeedsRouting } from "@/utils/providerCapabilities";

export interface ProviderSwitchSideEffectsParams {
  /** i18n 翻译函数（由调用方从 useTranslation 提供） */
  t: TFunction;
  /** 切换命令返回结果（用于「旧配置回填失败」警告） */
  switchResult?: SwitchResult;
}

/**
 * 悬浮球面板切换 provider 后补齐主窗口 useProviderActions.switchProvider 的副作用链，
 * 避免在两处复制 (a) Claude 插件联动、(b) 重启提示、(c) 代理必需警告 三处逻辑。
 *
 * 只做「提示 + 插件同步」等无阻断副作用，不负责真正切换（切换由调用方完成）。
 * 内部吞掉所有副作用错误：切换本身已成功，副作用失败不应影响面板收起。
 */
export async function applyProviderSwitchSideEffects(
  appId: AppId,
  providerId: string,
  params: ProviderSwitchSideEffectsParams,
): Promise<void> {
  const { t, switchResult } = params;

  // 悬浮球分组只带 id/name/icon，副作用判断需要完整 Provider（meta/category）。
  let provider: Provider | undefined;
  try {
    provider = (await providersApi.getAll(appId))[providerId];
  } catch (error) {
    console.warn("[floatingBall] 获取 provider 失败，跳过切换副作用", error);
  }

  // 代理运行/接管状态（用于判断是否需要弹代理警告）。
  let isProxyRunning = false;
  let isProxyTakeover = false;
  try {
    const [status, takeover] = await Promise.all([
      proxyApi.getProxyStatus(),
      proxyApi.getProxyTakeoverStatus(),
    ]);
    isProxyRunning = status?.running === true;
    isProxyTakeover = computeTakeoverActive(appId, takeover, isProxyRunning);
  } catch (error) {
    console.warn("[floatingBall] 获取代理状态失败，跳过代理必需警告", error);
  }

  // (c) 代理必需警告：需要本地路由却未就绪时提示（与主窗口判定一致）
  const proxyRequiredReason = provider
    ? computeProxyRequiredReason(
        appId,
        provider,
        isProxyRunning,
        isProxyTakeover,
        t,
      )
    : null;
  if (proxyRequiredReason) {
    toast.warning(
      t("notifications.proxyRequiredForSwitch", {
        reason: proxyRequiredReason,
        defaultValue:
          "此供应商{{reason}}，需要代理服务才能正常使用，请先启动代理",
      }),
    );
  }

  // (a) Claude 插件联动（失败在函数内部提示，不向上抛）
  if (provider) {
    await syncClaudePlugin(appId, provider, t);
  }

  // 旧供应商配置回填失败警告（与主窗口一致）
  if (switchResult?.warnings?.length) {
    toast.warning(
      t("notifications.backfillWarning", {
        defaultValue:
          "切换成功，但旧供应商配置回填失败，您手动修改的配置可能未保存",
      }),
      { duration: 5000 },
    );
  }

  // (b) 切换成功提示：悬浮球面板静默切换，不弹成功/重启 toast
  // （面板自动收起本身就是成功反馈；代理必需警告与回填失败警告仍保留）
}

/**
 * 计算目标应用是否处于代理接管态（与 App.tsx 的 isCurrentAppTakeoverActive 一致：
 * 只有代理应用才看接管开关；claude-desktop 的路由开关就是代理进程本身）。
 */
function computeTakeoverActive(
  appId: AppId,
  takeover: ProxyTakeoverStatus | undefined,
  isProxyRunning: boolean,
): boolean {
  if (!isProxyRunning || !takeover) return false;
  if (!isProxyAppId(appId)) return false;
  return (
    (takeover as unknown as Record<string, boolean | undefined>)[appId] === true
  );
}

/**
 * 计算「为什么这个供应商需要代理」的原因文案；无需路由时返回 null。
 * 判定链与 useProviderActions.switchProvider 保持一致（单一事实源之外的主窗口路径
 * 不在此改动，此处按同一语义复刻）。
 */
function computeProxyRequiredReason(
  appId: AppId,
  provider: Provider,
  isProxyRunning: boolean,
  isProxyTakeover: boolean,
  t: TFunction,
): string | null {
  const isCopilotProvider =
    appId === "claude" && provider.meta?.providerType === "github_copilot";
  const isCodexChatFormat =
    (appId === "codex" || appId === "grokbuild") &&
    (provider.meta?.apiFormat === "openai_chat" ||
      (typeof (provider.settingsConfig as Record<string, any>)?.config ===
        "string" &&
        isCodexChatWireApi(
          extractCodexWireApi(
            (provider.settingsConfig as Record<string, any>).config,
          ),
        )));
  const isCodexAnthropicFormat =
    (appId === "codex" || appId === "grokbuild") &&
    (provider.meta?.apiFormat === "anthropic" ||
      (typeof (provider.settingsConfig as Record<string, any>)?.config ===
        "string" &&
        isCodexAnthropicWireApi(
          extractCodexWireApi(
            (provider.settingsConfig as Record<string, any>).config,
          ),
        )));

  // Claude Desktop 的路由开关就是代理进程本身；其余应用还必须开启当前应用的
  // takeover。不能只看全局进程，否则其它应用已接管时会漏判；也不能只看 takeover，
  // 否则 Desktop 在路由已运行时会持续误报。
  const routingReady =
    appId === "claude-desktop"
      ? isProxyRunning === true
      : isProxyTakeover === true;

  let proxyRequiredReason: string | null = null;
  if (!routingReady && providerNeedsRouting(appId, provider)) {
    if (isCopilotProvider) {
      proxyRequiredReason = t("notifications.proxyReasonCopilot", {
        defaultValue: "使用 GitHub Copilot 作为 Claude 供应商",
      });
    } else if (isOAuthProviderType(provider.meta?.providerType)) {
      proxyRequiredReason = t("notifications.proxyReasonManagedOAuth", {
        defaultValue: "使用托管 OAuth 登录（令牌由本地路由注入）",
      });
    } else if (
      provider.meta?.apiFormat === "openai_chat" &&
      appId === "claude"
    ) {
      proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
        defaultValue: "使用 OpenAI Chat 接口格式",
      });
    } else if (
      provider.meta?.apiFormat === "openai_responses" &&
      appId === "claude"
    ) {
      proxyRequiredReason = t("notifications.proxyReasonOpenAIResponses", {
        defaultValue: "使用 OpenAI Responses 接口格式",
      });
    } else if (isCodexChatFormat) {
      proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
        defaultValue: "使用 OpenAI Chat 接口格式",
      });
    } else if (isCodexAnthropicFormat) {
      proxyRequiredReason = t("notifications.proxyReasonAnthropicMessages", {
        defaultValue: "使用 Anthropic Messages 接口格式",
      });
    } else if (
      appId === "claude-desktop" &&
      provider.meta?.claudeDesktopMode === "proxy"
    ) {
      proxyRequiredReason = t("notifications.proxyReasonClaudeDesktop", {
        defaultValue: "使用 Claude Desktop 本地路由模式",
      });
    } else if (
      provider.meta?.isFullUrl &&
      (appId === "claude" || appId === "codex" || appId === "grokbuild")
    ) {
      proxyRequiredReason = t("notifications.proxyReasonFullUrl", {
        defaultValue: "开启了完整 URL 连接模式",
      });
    } else {
      proxyRequiredReason = t("notifications.proxyReasonRoutingRequired", {
        defaultValue: "需要本地路由处理请求",
      });
    }
  }

  return proxyRequiredReason;
}

/**
 * Claude 插件联动：切换后把官方/非官方状态写入 ~/.claude/config.json。
 * 仅在 enableClaudePluginIntegration 开启且 app 为 claude 时执行（与主窗口一致）。
 */
async function syncClaudePlugin(
  appId: AppId,
  provider: Provider,
  t: TFunction,
): Promise<void> {
  if (appId !== "claude") return;

  try {
    const settings = await settingsApi.get();
    if (!settings?.enableClaudePluginIntegration) {
      return;
    }

    const isOfficial = provider.category === "official";
    await settingsApi.applyClaudePluginConfig({ official: isOfficial });

    // 静默执行，不显示成功通知
  } catch (error) {
    const detail =
      extractErrorMessage(error) ||
      t("notifications.syncClaudePluginFailed", {
        defaultValue: "同步 Claude 插件失败",
      });
    toast.error(detail, { duration: 4200 });
  }
}
