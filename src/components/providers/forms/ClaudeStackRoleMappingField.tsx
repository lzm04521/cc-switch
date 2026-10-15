import { useTranslation } from "react-i18next";

import { HelpTip } from "@/components/ui/help-tip";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import type { ClaudeStackModelRow } from "./ClaudeStackModelsField";
import { stripClaudeOneMMarker } from "./hooks/useModelState";

/** 四档别名映射的表单值：空串 = 跟随默认（该档用列表第一模型）。 */
export interface ClaudeStackRoleTiers {
  sonnet: string;
  opus: string;
  fable: string;
  haiku: string;
}

/** Radix Select 的选项值不能是空串，「跟随默认」用哨兵值占位。 */
const FOLLOW_DEFAULT_VALUE = "__follow_default__";

const ROLE_ROWS: Array<{
  tier: keyof ClaudeStackRoleTiers;
  labelKey: string;
  labelDefault: string;
}> = [
  {
    tier: "sonnet",
    labelKey: "providerForm.modelRoleSonnet",
    labelDefault: "Sonnet",
  },
  {
    tier: "opus",
    labelKey: "providerForm.modelRoleOpus",
    labelDefault: "Opus",
  },
  {
    tier: "fable",
    labelKey: "providerForm.modelRoleFable",
    labelDefault: "Fable",
  },
  {
    tier: "haiku",
    labelKey: "providerForm.modelRoleHaiku",
    labelDefault: "Haiku",
  },
];

/** 下拉选项用的模型行：去掉空行，值 = 去 1M 标记后的模型名。 */
export function claudeStackRoleOptions(rows: ClaudeStackModelRow[]): Array<{
  model: string;
  label: string;
}> {
  return rows
    .map((row) => {
      const model = stripClaudeOneMMarker(row.model).trim();
      return { model, label: row.displayName?.trim() || model };
    })
    .filter((option) => option.model !== "");
}

interface ClaudeStackRoleMappingFieldProps {
  rows: ClaudeStackModelRow[];
  values: ClaudeStackRoleTiers;
  onTierChange: (tier: keyof ClaudeStackRoleTiers, value: string) => void;
}

/**
 * 聚合模式默认分组的四档别名映射：把 Claude Code 的 Sonnet/Opus/Fable/Haiku 档请求指到
 * 本分组列表里的某个模型；「跟随默认」用默认模型（列表第一行）。数据存
 * `meta.stackRoleMapping`（后端 `stack::claude_route_roles`），与完整表单里 env 档位键的
 * 模型映射互不影响。仅 claude + 聚合布局 + 默认分组 + 列表 ≥2 模型时由表单渲染。
 */
export function ClaudeStackRoleMappingField({
  rows,
  values,
  onTierChange,
}: ClaudeStackRoleMappingFieldProps) {
  const { t } = useTranslation();
  const options = claudeStackRoleOptions(rows);
  const label = t("providerForm.stackRoleMappingLabel", {
    defaultValue: "模型映射",
  });

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-0.5">
        <span className="text-sm font-medium text-fg-1">{label}</span>
        <HelpTip title={label}>
          {t("providerForm.stackRoleMappingHint", {
            defaultValue:
              "把 Claude Code 的 Sonnet/Opus/Fable/Haiku 档请求映射到本分组列表里的模型；「跟随默认」用默认模型（列表第一行）。仅聚合模式的默认分组生效。",
          })}
        </HelpTip>
      </div>
      <div className="space-y-2">
        {ROLE_ROWS.map((role) => {
          const current = stripClaudeOneMMarker(values[role.tier]).trim();
          const matched = options.some((option) => option.model === current);
          const followDefault = t("providerForm.stackRoleMappingFollowDefault", {
            defaultValue: "跟随默认",
          });
          return (
            <div
              key={role.tier}
              className="grid grid-cols-1 gap-2 md:grid-cols-[120px_1fr]"
            >
              <div className="flex h-9 items-center rounded-md border border-input bg-subtle px-3 text-sm font-medium text-fg-2">
                {t(role.labelKey, { defaultValue: role.labelDefault })}
              </div>
              <Select
                value={matched ? current : FOLLOW_DEFAULT_VALUE}
                onValueChange={(value) =>
                  onTierChange(
                    role.tier,
                    value === FOLLOW_DEFAULT_VALUE ? "" : value,
                  )
                }
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={FOLLOW_DEFAULT_VALUE}>
                    {followDefault}
                  </SelectItem>
                  {options.map((option) => (
                    <SelectItem key={option.model} value={option.model}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          );
        })}
      </div>
    </div>
  );
}
