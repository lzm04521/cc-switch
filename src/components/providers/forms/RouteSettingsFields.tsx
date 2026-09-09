import { useTranslation } from "react-i18next";
import { Switch } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const ROUTE_KEY_PATTERN = /^[A-Za-z0-9._-]{1,100}$/;

export interface RouteKeyOwner {
  key: string;
  providerId: string;
}

/**
 * 路由 key 校验（前端预检，后端 ProviderService 仍 fail-fast 兜底）。
 * 返回 i18n key（错误文案）或 null（通过）。
 */
export function validateRouteKey(
  key: string,
  existingKeys: RouteKeyOwner[],
  currentProviderId?: string,
): string | null {
  const trimmed = key.trim();
  if (!ROUTE_KEY_PATTERN.test(trimmed)) {
    return "providers.form.route.keyInvalid";
  }
  if (trimmed.toLowerCase() === "default") {
    return "providers.form.route.keyReserved";
  }
  const conflict = existingKeys.some(
    (entry) =>
      entry.providerId !== currentProviderId &&
      entry.key.trim().toLowerCase() === trimmed.toLowerCase(),
  );
  if (conflict) {
    return "providers.form.route.keyDuplicate";
  }
  return null;
}

interface RouteSettingsFieldsProps {
  routeEnabled: boolean;
  routeKey: string;
  onChange: (next: { routeEnabled: boolean; routeKey: string }) => void;
  /** 同 app 内已启用路由的分组（含 key 与 provider id），重复预检用 */
  existingKeys: RouteKeyOwner[];
  /** 编辑模式下的当前 provider id（预检时排除自身） */
  currentProviderId?: string;
}

export function RouteSettingsFields({
  routeEnabled,
  routeKey,
  onChange,
  existingKeys,
  currentProviderId,
}: RouteSettingsFieldsProps) {
  const { t } = useTranslation();
  const errorKey = routeEnabled
    ? validateRouteKey(routeKey, existingKeys, currentProviderId)
    : null;

  return (
    <div className="space-y-2 border-t border-border-default pt-3">
      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <Label className="text-sm">
            {t("providers.form.route.title", {
              defaultValue: "加入会话级路由",
            })}
          </Label>
          <p className="text-xs text-muted-foreground">
            {t("providers.form.route.description", {
              defaultValue:
                '开启后可用 --model "<前缀><key>" 把请求路由到该分组（默认前缀 G.）',
            })}
          </p>
        </div>
        <Switch
          checked={routeEnabled}
          onCheckedChange={(checked) =>
            onChange({ routeEnabled: checked, routeKey })
          }
        />
      </div>
      {routeEnabled && (
        <div className="space-y-1">
          <Label className="text-xs" htmlFor="provider-route-key">
            {t("providers.form.route.keyLabel", { defaultValue: "路由 key" })}
          </Label>
          <Input
            id="provider-route-key"
            value={routeKey}
            placeholder={t("providers.form.route.keyPlaceholder", {
              defaultValue: "如 ds（字母/数字/._-，1–100 位）",
            })}
            onChange={(e) =>
              onChange({ routeEnabled, routeKey: e.target.value })
            }
          />
          {errorKey && (
            <p className="text-xs text-red-500">
              {t(errorKey, {
                defaultValue:
                  "路由 key 不合法（字母/数字/._-，1–100 位，同应用内唯一，default 为保留字）",
              })}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
