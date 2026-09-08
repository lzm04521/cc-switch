import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Route } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  routePrefixLikelyConflicts,
  validateRoutePrefixValue,
} from "@/lib/routePrefix";

interface RoutePrefixSettingsProps {
  routePrefix?: string;
  onAutoSave: (updates: { routePrefix?: string }) => Promise<boolean | void>;
}

export function RoutePrefixSettings({
  routePrefix,
  onAutoSave,
}: RoutePrefixSettingsProps) {
  const { t } = useTranslation();
  const [value, setValue] = useState(routePrefix ?? "");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    setValue(routePrefix ?? "");
  }, [routePrefix]);

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
    }
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
    </div>
  );
}
