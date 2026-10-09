import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Route } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  routePrefixLikelyConflicts,
  validateRoutePrefixValue,
} from "@/lib/routePrefix";

export type RouteModelsListMode = "groups" | "models" | "both";

interface RoutePrefixSettingsProps {
  routePrefix?: string;
  /** 聚合条目发布形态（fork：恢复旧体系三选一；缺省 both） */
  routeModelsMode?: RouteModelsListMode;
  /** 会话粘性跟随开关（fork 五项优化 D4；缺省开=保持存量行为） */
  routeStickySession?: boolean;
  onAutoSave: (updates: {
    routePrefix?: string;
    routeModelsEndpoint?: { mode: RouteModelsListMode };
    routeStickySession?: boolean;
  }) => Promise<boolean | void>;
}

/**
 * 聚合模式设置（fork：会话路由改造为聚合模式后，此面板承载聚合相关设置，
 * doc/20261009-设计文档 §9、实施计划-聚合模式五项优化）。三个子设置：
 * ① 聚合模型 id 前缀（默认 `ccs-`，与上游一致）；
 * ② 聚合模型列表内容（groups/models/both，作用于 /v1/models 系端点与 Codex 目录）；
 * ③ 粘性会话（关闭后不带聚合 id 的请求一律走默认成员）。
 */
export function RoutePrefixSettings({
  routePrefix,
  routeModelsMode,
  routeStickySession,
  onAutoSave,
}: RoutePrefixSettingsProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState(routePrefix ?? "");
  const [saved, setSaved] = useState(false);
  // 前缀变更提示：已选中的聚合模型 id 全部失效，需要重新选择
  const [changedNotice, setChangedNotice] = useState(false);
  const [mode, setMode] = useState<RouteModelsListMode>(
    routeModelsMode ?? "both",
  );
  const [sticky, setSticky] = useState(routeStickySession ?? true);

  useEffect(() => {
    setSticky(routeStickySession ?? true);
  }, [routeStickySession]);

  useEffect(() => {
    setValue(routePrefix ?? "");
  }, [routePrefix]);

  useEffect(() => {
    setMode(routeModelsMode ?? "both");
  }, [routeModelsMode]);

  const validation =
    value.trim() === ""
      ? { ok: true as const }
      : validateRoutePrefixValue(value);
  const conflictHint = value.trim() !== "" && routePrefixLikelyConflicts(value);

  const handleSave = async () => {
    if (!validation.ok) return;
    const next = value.trim() === "" ? undefined : value.trim();
    const ok = await onAutoSave({ routePrefix: next });
    if (ok !== false) {
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
      if ((next ?? "") !== (routePrefix ?? "")) {
        setChangedNotice(true);
      }
    }
  };

  const handleModeChange = async (next: RouteModelsListMode) => {
    if (next === mode) return;
    const ok = await onAutoSave({ routeModelsEndpoint: { mode: next } });
    if (ok !== false) {
      setMode(next);
    }
  };

  const handleStickyChange = async (next: boolean) => {
    const previous = sticky;
    setSticky(next);
    const ok = await onAutoSave({ routeStickySession: next });
    if (ok === false) {
      setSticky(previous); // 保存失败回滚（同 API 报文记录开关交互）
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Route className="h-4 w-4 text-blue-500" />
        <h4 className="text-sm font-semibold">
          {t("settings.advanced.routePrefix.title", {
            defaultValue: "聚合模型 id 前缀",
          })}
        </h4>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.advanced.routePrefix.description", {
          defaultValue:
            '聚合模式下模型 id 形如 "<前缀>claude.<分组>.<模型>"（Codex 为 "<前缀><分组>.<模型>"）；"<前缀>claude.<分组>" 短形式走该分组默认模型，"<前缀>claude.default" 解绑会话。留空恢复默认 ccs-。',
        })}
      </p>
      <div className="flex gap-2">
        <Input
          value={value}
          placeholder="ccs-"
          className="max-w-32"
          onChange={(e) => setValue(e.target.value)}
        />
        <Button
          size="sm"
          variant="outline"
          disabled={!validation.ok || value.trim() === (routePrefix ?? "")}
          onClick={() => void handleSave()}
        >
          {saved
            ? t("common.saved", { defaultValue: "已保存" })
            : t("common.save", { defaultValue: "保存" })}
        </Button>
      </div>
      {!validation.ok && (
        <p className="text-xs text-red-500">
          {t(`settings.advanced.routePrefix.invalid.${validation.reason}`, {
            defaultValue:
              "前缀须为 1–8 位可打印 ASCII（不含冒号与空白），且以非字母数字字符结尾（如 G.、@、ccs-）",
          })}
        </p>
      )}
      {conflictHint && validation.ok && (
        <p className="text-xs text-yellow-600 dark:text-yellow-400">
          {t("settings.advanced.routePrefix.conflictHint", {
            defaultValue:
              "提示：该前缀与常见模型名相近，可能产生误匹配；建议更换（如 G.、@）",
          })}
        </p>
      )}
      {changedNotice && (
        <p className="text-xs text-yellow-600 dark:text-yellow-400">
          {t("settings.advanced.routePrefix.changedNotice", {
            defaultValue:
              "前缀已变更：已选中的聚合模型 id 全部失效，请在客户端模型列表里重新选择。",
          })}
        </p>
      )}

      <div className="flex items-center gap-2 pt-1">
        <h4 className="text-sm font-semibold">
          {t("settings.advanced.routeModelsMode.title", {
            defaultValue: "聚合模型列表内容",
          })}
        </h4>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.advanced.routeModelsMode.description", {
          defaultValue:
            "控制 /v1/models 系端点与 Codex 模型目录发布哪些聚合条目。",
        })}
      </p>
      <Select
        value={mode}
        onValueChange={(next) =>
          void handleModeChange(next as RouteModelsListMode)
        }
      >
        <SelectTrigger className="max-w-64">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="both">
            {t("settings.advanced.routeModelsMode.both", {
              defaultValue: "分组 + 模型",
            })}
          </SelectItem>
          <SelectItem value="groups">
            {t("settings.advanced.routeModelsMode.groups", {
              defaultValue: "仅分组（短形式，选中走默认模型）",
            })}
          </SelectItem>
          <SelectItem value="models">
            {t("settings.advanced.routeModelsMode.models", {
              defaultValue: "仅模型（完整条目）",
            })}
          </SelectItem>
        </SelectContent>
      </Select>

      <div className="flex items-center justify-between gap-3 pt-1">
        <h4 className="text-sm font-semibold">
          {t("settings.advanced.routeStickySession.title", {
            defaultValue: "粘性会话",
          })}
        </h4>
        <Switch
          checked={sticky}
          onCheckedChange={(checked) => void handleStickyChange(checked)}
          aria-label={t("settings.advanced.routeStickySession.title", {
            defaultValue: "粘性会话",
          })}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.advanced.routeStickySession.description", {
          defaultValue:
            "开启后，会话中选中某个分组，该会话后续请求即使不带聚合模型 id 也继续走这个分组。例如 Claude Code 主会话选了智谱分组后，Task 工具拉起的 subagent 不指定模型，请求仍发给智谱，而不是回落到默认供应商。关闭后，不带聚合 id 的请求一律走当前默认供应商。",
        })}
      </p>
    </div>
  );
}
