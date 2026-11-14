import { useTranslation } from "react-i18next";
import { useState } from "react";
import type { ReactNode } from "react";
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { ImeSafeInput } from "@/components/ui/ime-safe-input";
import { HoverTip } from "@/components/ui/hover-tip";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { ProviderIconBox } from "@/components/ProviderIconBox";
import { IconPicker } from "@/components/IconPicker";
import { getIconMetadata } from "@/icons/extracted/metadata";
import { useWatch, type UseFormReturn } from "react-hook-form";
import type { ProviderFormData } from "@/lib/schemas/provider";

/**
 * 必填标签：红色「*」画在伪元素里（不进标签文字），输入框另带 aria-required 给读屏。
 */
export const REQUIRED_LABEL =
  "after:ms-0.5 after:text-danger-text after:content-['*']";

interface BasicFormFieldsProps {
  form: UseFormReturn<ProviderFormData>;
  /** Slot to render content between icon and name fields */
  beforeNameSlot?: ReactNode;
  /** fork：聚合布局——第一行 [名称|简称]、第二行 [分组 key|备注]；非聚合保持 [名称|备注] */
  stackLayout?: boolean;
  /** fork：分组 key 输入框节点（ProviderForm 生成，含归一化预览；仅已有成员非空） */
  stackKeyField?: ReactNode;
}

export function BasicFormFields({
  form,
  beforeNameSlot,
  stackLayout = false,
  stackKeyField,
}: BasicFormFieldsProps) {
  const { t } = useTranslation();
  const [iconDialogOpen, setIconDialogOpen] = useState(false);

  // useWatch 只让这里重渲染；form.watch 会让整张表单（ProviderForm）跟着每次按键重渲染
  const [currentIcon, currentIconColor, watchedName] = useWatch({
    control: form.control,
    name: ["icon", "iconColor", "name"],
  });
  const providerName = watchedName || "Provider";
  const effectiveIconColor =
    currentIconColor ||
    (currentIcon ? getIconMetadata(currentIcon)?.defaultColor : undefined);

  const handleIconSelect = (icon: string) => {
    const meta = getIconMetadata(icon);
    form.setValue("icon", icon);
    form.setValue("iconColor", meta?.defaultColor ?? "");
    // 单选：点了就是选好了，不用再找「完成」
    setIconDialogOpen(false);
  };

  const iconButton = (
    <>
      <HoverTip
        content={
          currentIcon
            ? t("providerIcon.clickToChange", {
                defaultValue: "点击更换图标",
              })
            : t("providerIcon.clickToSelect", {
                defaultValue: "点击选择图标",
              })
        }
      >
        <button
          type="button"
          onClick={() => setIconDialogOpen(true)}
          aria-label={t("providerIcon.change", { defaultValue: "更换图标" })}
          className="group shrink-0 rounded-[8px]"
        >
          <ProviderIconBox
            icon={currentIcon}
            name={providerName}
            color={effectiveIconColor}
            className="bg-surface transition-colors group-hover:bg-subtle"
          />
        </button>
      </HoverTip>
      <FullScreenPanel
        isOpen={iconDialogOpen}
        title={t("providerIcon.selectIcon", {
          defaultValue: "选择图标",
        })}
        onClose={() => setIconDialogOpen(false)}
      >
        <IconPicker
          value={currentIcon}
          onValueChange={handleIconSelect}
          color={effectiveIconColor}
          defaultName={watchedName ?? ""}
        />
      </FullScreenPanel>
    </>
  );

  const nameField = (
    <FormField
      control={form.control}
      name="name"
      render={({ field }) => (
        <FormItem>
          <FormLabel className={REQUIRED_LABEL}>{t("provider.name")}</FormLabel>
          <div className="flex items-center gap-2">
            {iconButton}
            <FormControl>
              <ImeSafeInput
                ref={field.ref}
                name={field.name}
                value={field.value ?? ""}
                onValueChange={field.onChange}
                onBlur={field.onBlur}
                disabled={field.disabled}
                placeholder={t("provider.namePlaceholder")}
                aria-required="true"
              />
            </FormControl>
          </div>
          <FormMessage />
        </FormItem>
      )}
    />
  );

  // fork：聚合显示名用的简称，仅聚合布局展示（后端 stack::provider_label 回退名称）
  const shortNameField = (
    <FormField
      control={form.control}
      name="shortName"
      render={({ field }) => (
        <FormItem>
          <FormLabel>{t("provider.shortName")}</FormLabel>
          <FormControl>
            <ImeSafeInput
              ref={field.ref}
              name={field.name}
              value={field.value ?? ""}
              onValueChange={field.onChange}
              onBlur={field.onBlur}
              disabled={field.disabled}
              placeholder={t("provider.shortNamePlaceholder")}
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  );

  const notesField = (
    <FormField
      control={form.control}
      name="notes"
      render={({ field }) => (
        <FormItem>
          <FormLabel>{t("provider.notes")}</FormLabel>
          <FormControl>
            <ImeSafeInput
              ref={field.ref}
              name={field.name}
              value={field.value ?? ""}
              onValueChange={field.onChange}
              onBlur={field.onBlur}
              disabled={field.disabled}
              placeholder={t("provider.notesPlaceholder")}
            />
          </FormControl>
          <FormMessage />
        </FormItem>
      )}
    />
  );

  return (
    <>
      {/* Slot for additional fields between icon and name */}
      {beforeNameSlot}

      {stackLayout ? (
        <>
          {/* 聚合布局：第一行 名称|简称；第二行 分组key|备注（key 缺席时占格保位） */}
          <div className="grid grid-cols-2 gap-4">
            {nameField}
            {shortNameField}
          </div>
          <div className="grid grid-cols-2 gap-4">
            {stackKeyField ?? <div aria-hidden="true" />}
            {notesField}
          </div>
        </>
      ) : (
        /* 基础信息 - 网格布局 */
        <div className="grid grid-cols-2 gap-4">
          {nameField}
          {notesField}
        </div>
      )}

      <FormField
        control={form.control}
        name="websiteUrl"
        render={({ field }) => (
          <FormItem>
            <FormLabel>{t("provider.websiteUrl")}</FormLabel>
            <FormControl>
              <ImeSafeInput
                ref={field.ref}
                name={field.name}
                value={field.value ?? ""}
                onValueChange={field.onChange}
                onBlur={field.onBlur}
                disabled={field.disabled}
                placeholder={t("providerForm.websiteUrlPlaceholder")}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
    </>
  );
}
