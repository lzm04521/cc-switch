import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Route } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  routePrefixLikelyConflicts,
  validateRoutePrefixValue,
} from "@/lib/routePrefix";

const MODE_OPTIONS = ["groups", "models", "both"] as const;
type ModeOption = (typeof MODE_OPTIONS)[number];

const MODE_LABELS: Record<ModeOption, string> = {
  groups: "仅分组",
  models: "模型",
  both: "分组+模型",
};

// 三个 models 端点各自的返回规则 + 关闭时的行为（面板列表式小字）
const ENDPOINT_RULES: ReadonlyArray<{ key: string; defaultValue: string }> = [
  {
    key: "settings.advanced.routeModelsEndpoint.endpointRules.claude",
    defaultValue: "/claude/v1/models：仅返回 claude 分组条目",
  },
  {
    key: "settings.advanced.routeModelsEndpoint.endpointRules.codex",
    defaultValue:
      "/codex/v1/models：在 Codex 模型目录基础上追加 codex 分组条目与模型映射清单",
  },
  {
    key: "settings.advanced.routeModelsEndpoint.endpointRules.v1",
    defaultValue: "/v1/models：Codex 目录 + claude 分组条目",
  },
  {
    key: "settings.advanced.routeModelsEndpoint.endpointRules.disabled",
    defaultValue: "关闭时三个端点均不含路由条目",
  },
];

interface RoutePrefixSettingsProps {
  routePrefix?: string;
  routeModelsEndpoint?: { enabled: boolean; mode: ModeOption };
  onAutoSave: (updates: {
    routePrefix?: string;
    routeModelsEndpoint?: { enabled: boolean; mode: ModeOption };
  }) => Promise<boolean | void>;
}

export function RoutePrefixSettings({
  routePrefix,
  routeModelsEndpoint,
  onAutoSave,
}: RoutePrefixSettingsProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState(routePrefix ?? "");
  const [saved, setSaved] = useState(false);
  const [endpointSaved, setEndpointSaved] = useState(false);

  useEffect(() => {
    setValue(routePrefix ?? "");
  }, [routePrefix]);

  const endpointEnabled = routeModelsEndpoint?.enabled ?? false;
  const endpointMode: ModeOption = routeModelsEndpoint?.mode ?? "groups";

  const validation =
    value.trim() === ""
      ? { ok: true as const }
      : validateRoutePrefixValue(value);
  const conflictHint = value.trim() !== "" && routePrefixLikelyConflicts(value);

  const flashSaved = () => {
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };

  // 模型列表接口区块独立的保存反馈，不复用前缀保存按钮的 saved 状态
  const flashEndpointSaved = () => {
    setEndpointSaved(true);
    setTimeout(() => setEndpointSaved(false), 1500);
  };

  const handleSave = async () => {
    if (!validation.ok) return;
    const next = value.trim() === "" ? undefined : value.trim();
    const ok = await onAutoSave({ routePrefix: next });
    if (ok !== false) flashSaved();
  };

  const saveEndpoint = async (next: { enabled: boolean; mode: ModeOption }) => {
    const ok = await onAutoSave({ routeModelsEndpoint: next });
    if (ok !== false) flashEndpointSaved();
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <Route className="h-4 w-4 text-blue-500" />
        <h4 className="text-sm font-semibold">
          {t("settings.advanced.routePrefix.title", {
            defaultValue: "会话级路由前缀",
          })}
        </h4>
      </div>
      <p className="text-xs text-muted-foreground">
        {t("settings.advanced.routePrefix.description", {
          defaultValue:
            '供应商开启「加入会话级路由」后，用 --model "<前缀><key>" 将请求路由到该分组；<前缀>default 解绑当前会话。留空恢复默认 G.。',
        })}
      </p>
      <div className="flex gap-2">
        <Input
          value={value}
          placeholder="G."
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
              "前缀须为 1–8 位可打印 ASCII（不含冒号与空白），且以非字母数字字符结尾（如 G.、@）",
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

      {/* /v1/models 路由模型列表（设计 §4.5） */}
      <div className="space-y-2 border-t border-border-default pt-3">
        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <Label className="text-sm">
              {t("settings.advanced.routeModelsEndpoint.title", {
                defaultValue: "/v1/models 模型列表",
              })}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t("settings.advanced.routeModelsEndpoint.description", {
                defaultValue:
                  "开启后，路由分组条目会出现在 /claude/v1/models、/codex/v1/models、/v1/models 三个端点；仅显示已加入会话级路由的分组，条目 id 可直接用于 --model（如 G.DS、G.DS:glm-5.3）。",
              })}
            </p>
          </div>
          <div className="flex items-center gap-2">
            {endpointSaved && (
              <span className="text-xs text-muted-foreground">
                {t("common.saved", { defaultValue: "已保存" })}
              </span>
            )}
            <Switch
              checked={endpointEnabled}
              onCheckedChange={(checked) =>
                void saveEndpoint({ enabled: checked, mode: endpointMode })
              }
            />
          </div>
        </div>
        <ul className="list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
          {ENDPOINT_RULES.map((rule) => (
            <li key={rule.key}>
              {t(rule.key, { defaultValue: rule.defaultValue })}
            </li>
          ))}
        </ul>
        {endpointEnabled && (
          <div className="space-y-1">
            <Label className="text-xs">
              {t("settings.advanced.routeModelsEndpoint.modeLabel", {
                defaultValue: "返回类型",
              })}
            </Label>
            <div className="flex gap-1">
              {MODE_OPTIONS.map((option) => (
                <Button
                  key={option}
                  size="sm"
                  variant={option === endpointMode ? "default" : "outline"}
                  onClick={() =>
                    void saveEndpoint({ enabled: true, mode: option })
                  }
                >
                  {t(`settings.advanced.routeModelsEndpoint.mode.${option}`, {
                    defaultValue: MODE_LABELS[option],
                  })}
                </Button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
