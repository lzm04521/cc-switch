import { useTranslation } from "react-i18next";
import { Download, Loader2, Plus, Star, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { Checkbox } from "@/components/ui/checkbox";
import { FormLabel } from "@/components/ui/form";
import { ImeSafeInput } from "@/components/ui/ime-safe-input";
import { Input } from "@/components/ui/input";
import { useSettingsQuery } from "@/lib/query";
import { resolveDisplayRoutePrefix } from "@/lib/routePrefix";
import type { FetchedModel } from "@/lib/api/model-fetch";
import type { ClaudeStackModel } from "@/types";
import { FetchedModelPicker } from "./FetchedModelPicker";
import { ModelDropdown } from "./shared";
import {
  hasClaudeOneMMarker,
  stripClaudeOneMMarker,
} from "./hooks/useModelState";

/** 表单里的一行：多一个只在前端用的行 id，删行时输入框不串位。 */
export type ClaudeStackModelRow = ClaudeStackModel & { rowId: string };

let nextRowId = 0;

export function createClaudeStackModelRow(
  seed?: Partial<ClaudeStackModel>,
): ClaudeStackModelRow {
  nextRowId += 1;
  return { model: "", ...seed, rowId: `stack-model-${nextRowId}` };
}

/**
 * 存进 `meta.stackModels` 的样子：去掉空行，模型名里写的 `[1M]` 换成 `oneM`，同名的合并
 * （有一处勾了 1M 就按 1M，显示名取第一个填了的）。
 */
export function normalizeClaudeStackModels(
  rows: ClaudeStackModel[],
): ClaudeStackModel[] {
  const result: ClaudeStackModel[] = [];
  for (const row of rows) {
    const raw = row.model.trim();
    const model = stripClaudeOneMMarker(raw).trim();
    if (!model) continue;
    const oneM = row.oneM === true || hasClaudeOneMMarker(raw);
    const displayName = row.displayName?.trim() || undefined;
    const existing = result.find((entry) => entry.model === model);
    if (existing) {
      if (oneM) existing.oneM = true;
      if (!existing.displayName && displayName) {
        existing.displayName = displayName;
      }
      continue;
    }
    result.push({
      model,
      ...(displayName ? { displayName } : {}),
      ...(oneM ? { oneM: true } : {}),
    });
  }
  return result;
}

/** 模型映射里的字段：`ANTHROPIC_MODEL` 和各档，按后端 `mode::stack::mapped_models` 的顺序。 */
const MAPPED_MODEL_FIELDS: Array<[string, string | null]> = [
  ["ANTHROPIC_MODEL", null],
  ["ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"],
  ["ANTHROPIC_DEFAULT_SONNET_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"],
  ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"],
  ["ANTHROPIC_DEFAULT_FABLE_MODEL", "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"],
];

/**
 * 没配列表时后端发布的模型（和 `mode::stack::mapped_models` 一致）：模型映射里的主模型和
 * 各档，按去掉 1M 标记后的名字去重，有一处带标记就按 1M，显示名取对应档位的。
 */
export function claudeStackModelsFromEnv(
  env: Record<string, unknown> | undefined,
): ClaudeStackModel[] {
  const rows: ClaudeStackModel[] = [];
  for (const [modelKey, nameKey] of MAPPED_MODEL_FIELDS) {
    const raw = env?.[modelKey];
    if (typeof raw !== "string") continue;
    const nameRaw = nameKey ? env?.[nameKey] : undefined;
    rows.push({
      model: raw,
      displayName: typeof nameRaw === "string" ? nameRaw : undefined,
    });
  }
  return normalizeClaudeStackModels(rows);
}

interface ClaudeStackModelsFieldProps {
  rows: ClaudeStackModelRow[];
  onRowsChange: (rows: ClaudeStackModelRow[]) => void;
  /** 从上游取到的模型：批量勾选和每行的下拉都用它。 */
  fetchedModels: FetchedModel[];
  onFetchModels: () => void;
  isFetchingModels: boolean;
  /** fork 五项优化：分组 key（归一化后的预览值），只读列生成聚合 id 用；缺省不显示预览。 */
  previewKey?: string;
  /** fork 五项优化：供应商名（表单当前名称），只读列显示名用。 */
  providerName?: string;
}

/**
 * Stack 模式下这家发布到 Claude Code 模型选择器里的模型。第一个是这家的默认模型：这家被
 * 设为默认时，Claude Code 启动、后台任务和子代理别名都用它。
 */
export function ClaudeStackModelsField({
  rows,
  onRowsChange,
  fetchedModels,
  onFetchModels,
  isFetchingModels,
  previewKey,
  providerName,
}: ClaudeStackModelsFieldProps) {
  const { t } = useTranslation();
  // fork 五项优化：只读预览列的前缀取设置（react-query 缓存共享，无额外请求），
  // 经 resolveDisplayRoutePrefix 归一化——与代理实际生效的前缀一致
  const { data: settings } = useSettingsQuery();
  const previewPrefix = previewKey
    ? resolveDisplayRoutePrefix(settings?.routePrefix)
    : "";
  const showPreviewColumns = Boolean(previewKey);

  /** 与后端 stack::encode 一致的聚合模型 id（Claude：`<prefix>claude.<key>.<model>[1M]?`）。 */
  const selectorId = (row: ClaudeStackModelRow): string => {
    const model = stripClaudeOneMMarker(row.model).trim();
    if (!model) return "";
    const oneM = row.oneM === true || hasClaudeOneMMarker(row.model);
    return `${previewPrefix}claude.${previewKey}.${model}${oneM ? "[1M]" : ""}`;
  };

  /** 与后端 stack::display_name 一致的选择器显示名（`<显示名|模型名>（<供应商名>）`）。 */
  const selectorName = (row: ClaudeStackModelRow): string => {
    const model = stripClaudeOneMMarker(row.model).trim();
    if (!model) return "";
    return `${row.displayName?.trim() || model}（${providerName ?? ""}）`;
  };

  const updateRow = (rowId: string, patch: Partial<ClaudeStackModel>) =>
    onRowsChange(
      rows.map((row) => (row.rowId === rowId ? { ...row, ...patch } : row)),
    );

  const addFetchedModels = (modelIds: string[]) => {
    const configured = new Set(
      rows.map((row) => stripClaudeOneMMarker(row.model).trim()),
    );
    const additions = modelIds
      .filter((id) => !configured.has(id))
      .map((id) => createClaudeStackModelRow({ model: id }));
    onRowsChange([...rows, ...additions]);
  };

  const makeDefault = (rowId: string) => {
    const row = rows.find((other) => other.rowId === rowId);
    if (!row) return;
    onRowsChange([row, ...rows.filter((other) => other.rowId !== rowId)]);
  };

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <div className="flex items-center justify-between gap-3">
          <FormLabel>
            {t("providerForm.stackModelsLabel", { defaultValue: "模型列表" })}
          </FormLabel>
          <div className="flex gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onFetchModels}
              disabled={isFetchingModels}
              className="h-7 gap-1"
            >
              {isFetchingModels ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Download className="h-3.5 w-3.5" />
              )}
              {t("providerForm.fetchModels")}
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() =>
                onRowsChange([...rows, createClaudeStackModelRow()])
              }
              className="h-7 gap-1"
            >
              <Plus className="h-3.5 w-3.5" />
              {t("providerForm.addStackModel", { defaultValue: "手动添加" })}
            </Button>
          </div>
        </div>
        <p className="text-xs leading-relaxed text-fg-2">
          {t("providerForm.stackModelsHint", {
            defaultValue:
              "这些模型会出现在 Claude Code 的 /model 里，选中后请求直达这家。第一个（★）是这家的默认模型：这家被设为默认时，Claude Code 启动和后台任务都用它。",
          })}
        </p>
      </div>

      {fetchedModels.length > 0 && (
        <FetchedModelPicker
          models={fetchedModels}
          configuredModelIds={rows.map((row) =>
            stripClaudeOneMMarker(row.model).trim(),
          )}
          onAdd={addFetchedModels}
        />
      )}

      {rows.length === 0 ? (
        <p className="text-xs leading-relaxed text-fg-2">
          {t("providerForm.stackModelsEmpty", {
            defaultValue:
              "未配置模型：把这家加入聚合后，模型选择器里不会多出它的模型",
          })}
        </p>
      ) : (
        <div className="space-y-2">
          {/* fork 五项优化：最前两列只读预览（选择器 id / 选择器显示名），有分组 key
              才显示；窄列 truncate + title 悬停看全 */}
          <div
            className={cn(
              "hidden gap-2 px-1 text-xs font-medium text-fg-2 md:grid",
              showPreviewColumns
                ? "grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_36px_1fr_minmax(0,1fr)_64px_36px]"
                : "grid-cols-[36px_1fr_minmax(0,1fr)_64px_36px]",
            )}
          >
            {showPreviewColumns && (
              <>
                <span>
                  {t("providerForm.selectorIdLabel", {
                    defaultValue: "选择器 id",
                  })}
                </span>
                <span>
                  {t("providerForm.selectorNameLabel", {
                    defaultValue: "选择器显示名",
                  })}
                </span>
              </>
            )}
            <span />
            <span>
              {t("providerForm.modelDisplayNameLabel", {
                defaultValue: "显示名称",
              })}
            </span>
            <span>
              {t("providerForm.requestModelLabel", {
                defaultValue: "实际请求模型",
              })}
            </span>
            <span>
              {t("providerForm.modelOneMLabel", { defaultValue: "1M" })}
            </span>
            <span />
          </div>
          {rows.map((row, index) => {
            const modelBase = stripClaudeOneMMarker(row.model).trim();
            const isDefault = index === 0;
            const defaultLabel = isDefault
              ? t("providerForm.stackModelDefault", {
                  defaultValue: "默认模型",
                })
              : t("providerForm.stackModelMakeDefault", {
                  defaultValue: "设为默认模型",
                });
            return (
              <div
                key={row.rowId}
                className={cn(
                  "grid grid-cols-1 gap-2",
                  showPreviewColumns
                    ? "md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_36px_1fr_minmax(0,1fr)_64px_36px]"
                    : "md:grid-cols-[36px_1fr_minmax(0,1fr)_64px_36px]",
                )}
              >
                {showPreviewColumns && (
                  <>
                    <div
                      className="flex h-9 min-w-0 items-center"
                      title={selectorId(row)}
                    >
                      <span className="truncate text-xs text-fg-2">
                        {selectorId(row) || "—"}
                      </span>
                    </div>
                    <div
                      className="flex h-9 min-w-0 items-center"
                      title={selectorName(row)}
                    >
                      <span className="truncate text-xs text-fg-2">
                        {selectorName(row) || "—"}
                      </span>
                    </div>
                  </>
                )}
                <HoverTip content={defaultLabel}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() => makeDefault(row.rowId)}
                    disabled={isDefault}
                    aria-label={defaultLabel}
                    aria-pressed={isDefault}
                    className="h-9 w-9 text-fg-2 hover:text-warning-text disabled:opacity-100"
                  >
                    <Star
                      className={
                        isDefault
                          ? "h-4 w-4 fill-warning text-warning-text"
                          : "h-4 w-4"
                      }
                    />
                  </Button>
                </HoverTip>
                <ImeSafeInput
                  value={row.displayName ?? ""}
                  onValueChange={(value) =>
                    updateRow(row.rowId, { displayName: value })
                  }
                  placeholder={
                    modelBase ||
                    t("providerForm.modelDisplayNamePlaceholder", {
                      defaultValue: "例如 DeepSeek V4 Pro",
                    })
                  }
                  aria-label={t("providerForm.modelDisplayNameLabel", {
                    defaultValue: "显示名称",
                  })}
                  autoComplete="off"
                />
                <div className="flex gap-1">
                  <Input
                    value={row.model}
                    onChange={(event) =>
                      updateRow(row.rowId, { model: event.target.value })
                    }
                    placeholder={t("providerForm.stackModelPlaceholder", {
                      defaultValue: "例如 deepseek-v4-pro",
                    })}
                    aria-label={t("providerForm.requestModelLabel", {
                      defaultValue: "实际请求模型",
                    })}
                    autoComplete="off"
                    className="flex-1"
                  />
                  {fetchedModels.length > 0 && (
                    <ModelDropdown
                      models={fetchedModels}
                      onSelect={(id) => updateRow(row.rowId, { model: id })}
                    />
                  )}
                </div>
                <label className="flex h-9 items-center gap-2 text-sm text-fg-2">
                  <Checkbox
                    checked={
                      row.oneM === true || hasClaudeOneMMarker(row.model)
                    }
                    onCheckedChange={(checked) =>
                      updateRow(row.rowId, {
                        model: stripClaudeOneMMarker(row.model),
                        oneM: checked === true,
                      })
                    }
                  />
                  {t("providerForm.modelOneMLabel", { defaultValue: "1M" })}
                </label>
                <HoverTip content={t("common.delete")}>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={() =>
                      onRowsChange(
                        rows.filter((other) => other.rowId !== row.rowId),
                      )
                    }
                    aria-label={t("common.delete")}
                    className="h-9 w-9 text-fg-2 hover:text-destructive"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </HoverTip>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
