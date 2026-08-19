import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { type AppId } from "@/lib/api";
import { usePromptActions } from "@/hooks/usePromptActions";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from "@/components/ui/select";
import { ProviderIcon } from "@/components/ProviderIcon";
import PiPromptPanel, { type PromptPrimaryAction } from "./PiPromptPanel";
import PromptFormPanel from "./PromptFormPanel";
import { PromptLibrary } from "./PromptLibrary";
import { ConfirmDialog } from "../ConfirmDialog";

interface PromptPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  appId: AppId;
  onInteractionBlockedChange?: (blocked: boolean) => void;
  onNavigationBlockedChange?: (blocked: boolean) => void;
  onPrimaryActionChange?: (action: PromptPrimaryAction) => void;
  // 页内切换应用后上报（null 表示面板卸载，标题回退到外部 appId）
  onSelectedAppChange?: (appId: AppId | null) => void;
}

export interface PromptPanelHandle {
  openAdd: () => void;
}

export type { PromptPrimaryAction } from "./PiPromptPanel";

// 提示词页面支持切换的 agent 列表（Pi 有独立模板面板，经主侧栏进入）
const PROMPT_APP_OPTIONS: Array<{
  value: AppId;
  icon: string;
  labelKey: string;
}> = [
  { value: "claude", icon: "claude", labelKey: "apps.claudeCode" },
  { value: "codex", icon: "openai", labelKey: "apps.codex" },
  { value: "gemini", icon: "gemini", labelKey: "apps.gemini" },
  { value: "grokbuild", icon: "grok", labelKey: "apps.grokbuild" },
  { value: "opencode", icon: "opencode", labelKey: "apps.opencode" },
  { value: "openclaw", icon: "openclaw", labelKey: "apps.openclaw" },
  { value: "hermes", icon: "hermes", labelKey: "apps.hermes" },
  { value: "zcode", icon: "zcode", labelKey: "apps.zcode" },
];

const StandardPromptPanel = React.forwardRef<
  PromptPanelHandle,
  PromptPanelProps
>(
  (
    {
      open,
      appId,
      onInteractionBlockedChange,
      onNavigationBlockedChange,
      onPrimaryActionChange,
      onSelectedAppChange,
    },
    ref,
  ) => {
    const { t } = useTranslation();
    // 面板内应用切换（polish 版行为）：默认跟随外部 appId，可在页内切换
    const [selectedAppId, setSelectedAppId] = useState<AppId>(appId);
    useEffect(() => {
      setSelectedAppId(appId);
    }, [appId]);
    const [isFormOpen, setIsFormOpen] = useState(false);
    const [editingId, setEditingId] = useState<string | null>(null);
    const [searchQuery, setSearchQuery] = useState("");
    const [confirmDialog, setConfirmDialog] = useState<{
      isOpen: boolean;
      titleKey: string;
      messageKey: string;
      messageParams?: Record<string, unknown>;
      onConfirm: () => void;
    } | null>(null);
    const [writePending, setWritePending] = useState(false);
    const [reloadPending, setReloadPending] = useState(false);
    const writeLockRef = React.useRef(false);
    const reloadLockRef = React.useRef(false);
    const reloadRunGenerationRef = React.useRef(0);
    const overlayOpenRef = React.useRef(false);
    const externalReloadQueuedRef = React.useRef(false);

    const {
      prompts,
      loading,
      reload,
      savePrompt,
      deletePrompt,
      toggleEnabled,
    } = usePromptActions(selectedAppId);
    const reloadRef = React.useRef(reload);
    reloadRef.current = reload;

    const dialogOpen = confirmDialog !== null;
    const interactionBlocked =
      loading || reloadPending || writePending || isFormOpen || dialogOpen;
    const navigationBlocked = writePending || isFormOpen || dialogOpen;

    useEffect(() => {
      onInteractionBlockedChange?.(interactionBlocked);
    }, [interactionBlocked, onInteractionBlockedChange]);

    useEffect(() => {
      onNavigationBlockedChange?.(navigationBlocked);
    }, [navigationBlocked, onNavigationBlockedChange]);

    useEffect(() => {
      onPrimaryActionChange?.("prompt");
    }, [onPrimaryActionChange]);

    // 标题跟随页内切换的应用；卸载时置空，外部回退到自己的 appId
    useEffect(() => {
      onSelectedAppChange?.(selectedAppId);
    }, [selectedAppId, onSelectedAppChange]);

    useEffect(
      () => () => {
        onInteractionBlockedChange?.(false);
        onNavigationBlockedChange?.(false);
        onSelectedAppChange?.(null);
      },
      [
        onInteractionBlockedChange,
        onNavigationBlockedChange,
        onSelectedAppChange,
      ],
    );

    const runExternalReload = React.useCallback(async () => {
      if (writeLockRef.current || overlayOpenRef.current) {
        externalReloadQueuedRef.current = true;
        return;
      }

      const runGeneration = ++reloadRunGenerationRef.current;
      externalReloadQueuedRef.current = false;
      reloadLockRef.current = true;
      setReloadPending(true);
      try {
        await reloadRef.current();
      } finally {
        if (reloadRunGenerationRef.current === runGeneration) {
          reloadLockRef.current = false;
          setReloadPending(false);
        }
      }
    }, []);

    const beginWrite = () => {
      if (loading || reloadLockRef.current || writeLockRef.current)
        return false;
      writeLockRef.current = true;
      setWritePending(true);
      return true;
    };

    const endWrite = () => {
      writeLockRef.current = false;
      setWritePending(false);
      if (externalReloadQueuedRef.current) {
        void runExternalReload();
      }
    };

    useEffect(() => {
      if (open) void runExternalReload();
    }, [selectedAppId, open, runExternalReload]);

    useEffect(() => {
      setSearchQuery("");
      overlayOpenRef.current = false;
      setIsFormOpen(false);
      setEditingId(null);
      setConfirmDialog(null);
      if (externalReloadQueuedRef.current) {
        void runExternalReload();
      }
    }, [selectedAppId, runExternalReload]);

    useEffect(() => {
      const handlePromptImported = (event: Event) => {
        const customEvent = event as CustomEvent;
        if (customEvent.detail?.app === selectedAppId) {
          void runExternalReload();
        }
      };

      window.addEventListener("prompt-imported", handlePromptImported);
      return () => {
        window.removeEventListener("prompt-imported", handlePromptImported);
      };
    }, [selectedAppId, runExternalReload]);

    useTauriEvent("profile-applied", runExternalReload);

    const handleAdd = () => {
      if (reloadLockRef.current || writeLockRef.current || interactionBlocked) {
        return;
      }
      overlayOpenRef.current = true;
      setEditingId(null);
      setIsFormOpen(true);
    };

    React.useImperativeHandle(ref, () => ({
      openAdd: handleAdd,
    }));

    const handleEdit = (id: string) => {
      if (reloadLockRef.current || writeLockRef.current || interactionBlocked) {
        return;
      }
      overlayOpenRef.current = true;
      setEditingId(id);
      setIsFormOpen(true);
    };

    const handleDelete = (id: string) => {
      if (reloadLockRef.current || writeLockRef.current || interactionBlocked) {
        return;
      }
      const prompt = prompts[id];
      overlayOpenRef.current = true;
      setConfirmDialog({
        isOpen: true,
        titleKey: "prompts.confirm.deleteTitle",
        messageKey: "prompts.confirm.deleteMessage",
        messageParams: { name: prompt?.name },
        onConfirm: async () => {
          if (!beginWrite()) return;
          try {
            const refreshed = await deletePrompt(id);
            if (refreshed === false) {
              externalReloadQueuedRef.current = true;
            }
            overlayOpenRef.current = false;
            setConfirmDialog(null);
          } catch {
            // Error handled by hook
          } finally {
            endWrite();
          }
        },
      });
    };

    const handleToggle = async (id: string, enabled: boolean) => {
      if (!beginWrite()) return;
      try {
        const refreshed = await toggleEnabled(id, enabled);
        if (refreshed === false) {
          externalReloadQueuedRef.current = true;
        }
      } catch {
        // Error handled by hook
      } finally {
        endWrite();
      }
    };

    const handleSave = async (
      id: string,
      prompt: Parameters<typeof savePrompt>[1],
    ) => {
      if (!beginWrite()) return false;
      try {
        const refreshed = await savePrompt(id, prompt);
        if (refreshed === false) {
          externalReloadQueuedRef.current = true;
        }
        return true;
      } catch {
        // Error handled by hook
        return false;
      } finally {
        endWrite();
      }
    };

    const handleCloseForm = () => {
      if (writeLockRef.current) return;
      overlayOpenRef.current = false;
      setIsFormOpen(false);
      setEditingId(null);
      if (externalReloadQueuedRef.current) {
        void runExternalReload();
      }
    };

    const promptEntries = Object.entries(prompts);
    const enabledPrompt = promptEntries.find(([, prompt]) => prompt.enabled);

    return (
      <div className="flex flex-col flex-1 min-h-0 px-6">
        {/* 应用切换下拉框靠右，与页头右侧操作区对齐 */}
        <div className="flex-shrink-0 pt-4 pb-2 flex justify-end">
          <Select
            value={selectedAppId}
            onValueChange={(value) => setSelectedAppId(value as AppId)}
          >
            <SelectTrigger
              className="h-8 w-auto gap-1.5 border-border-default bg-background text-sm"
              aria-label={t("prompts.appFilterTooltip")}
            >
              <ProviderIcon
                icon={
                  PROMPT_APP_OPTIONS.find((opt) => opt.value === selectedAppId)
                    ?.icon ?? "claude"
                }
                name={selectedAppId}
                size={14}
              />
              <span>
                {t(
                  PROMPT_APP_OPTIONS.find((opt) => opt.value === selectedAppId)
                    ?.labelKey ?? "apps.claudeCode",
                )}
              </span>
            </SelectTrigger>
            <SelectContent>
              {PROMPT_APP_OPTIONS.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  <div className="flex items-center gap-2">
                    <ProviderIcon icon={opt.icon} name={opt.value} size={14} />
                    <span>{t(opt.labelKey)}</span>
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <PromptLibrary
          prompts={prompts}
          loading={loading}
          searchQuery={searchQuery}
          statusText={
            enabledPrompt
              ? t("prompts.enabledName", { name: enabledPrompt[1].name })
              : t("prompts.noneEnabled")
          }
          disabled={interactionBlocked}
          onSearchQueryChange={setSearchQuery}
          onToggle={handleToggle}
          onEdit={handleEdit}
          onDelete={handleDelete}
        />

        {isFormOpen && (
          <PromptFormPanel
            appId={selectedAppId}
            editingId={editingId || undefined}
            initialData={editingId ? prompts[editingId] : undefined}
            onSave={handleSave}
            onClose={handleCloseForm}
          />
        )}

        {confirmDialog && (
          <ConfirmDialog
            isOpen={confirmDialog.isOpen}
            title={t(confirmDialog.titleKey)}
            message={t(confirmDialog.messageKey, confirmDialog.messageParams)}
            pending={writePending}
            onConfirm={confirmDialog.onConfirm}
            onCancel={() => {
              if (!writeLockRef.current) {
                overlayOpenRef.current = false;
                setConfirmDialog(null);
                if (externalReloadQueuedRef.current) {
                  void runExternalReload();
                }
              }
            }}
          />
        )}
      </div>
    );
  },
);

StandardPromptPanel.displayName = "StandardPromptPanel";

const PromptPanel = React.forwardRef<PromptPanelHandle, PromptPanelProps>(
  (props, ref) => {
    if (props.appId === "pi") {
      return (
        <PiPromptPanel
          ref={ref}
          open={props.open}
          onInteractionBlockedChange={props.onInteractionBlockedChange}
          onNavigationBlockedChange={props.onNavigationBlockedChange}
          onPrimaryActionChange={props.onPrimaryActionChange}
        />
      );
    }

    return <StandardPromptPanel ref={ref} {...props} />;
  },
);

PromptPanel.displayName = "PromptPanel";

export default PromptPanel;
