import { useTranslation } from "react-i18next";
import { FormLabel } from "@/components/ui/form";
import { Textarea } from "@/components/ui/textarea";
import { HelpTip } from "@/components/ui/help-tip";
import {
  parseBodyOverrideJson,
  parseBodyRewritesJson,
  parseHeaderOverrideJson,
} from "@/lib/requestOverrides";

interface LocalProxyRequestOverridesFieldProps {
  headersJson: string;
  bodyJson: string;
  rewritesJson: string;
  onHeadersJsonChange: (value: string) => void;
  onBodyJsonChange: (value: string) => void;
  /** 字段名后「?」里的说明；「只在路由、聚合模式下生效」由调用方在分组开头说一次。 */
  help: string;
  onRewritesJsonChange: (value: string) => void;
}

export function LocalProxyRequestOverridesField({
  headersJson,
  bodyJson,
  rewritesJson,
  onHeadersJsonChange,
  onBodyJsonChange,
  help,
  onRewritesJsonChange,
}: LocalProxyRequestOverridesFieldProps) {
  const { t } = useTranslation();
  const label = t("providerForm.localProxyRequestOverrides", {
    defaultValue: "本地代理请求覆盖",
  });
  const headerError = parseHeaderOverrideJson(headersJson).error;
  const bodyError = parseBodyOverrideJson(bodyJson).error;
  const rewritesError = parseBodyRewritesJson(rewritesJson).error;

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-0.5">
        <FormLabel>{label}</FormLabel>
        <HelpTip title={label}>{help}</HelpTip>
      </div>

      <div className="grid gap-3 md:grid-cols-2">
        <div className="space-y-2">
          <FormLabel className="text-xs text-fg-2">
            {t("providerForm.localProxyHeaderOverrides", {
              defaultValue: "Header 覆盖",
            })}
          </FormLabel>
          <Textarea
            value={headersJson}
            onChange={(event) => onHeadersJsonChange(event.target.value)}
            placeholder={'{\n  "X-Provider": "cc-switch"\n}'}
            className="min-h-[132px] resize-y font-mono text-xs"
            aria-invalid={Boolean(headerError)}
          />
          {headerError && (
            <p className="text-xs text-destructive">
              {t("providerForm.localProxyHeaderOverridesInvalidDetail", {
                error: headerError,
                defaultValue: "Header 覆盖格式错误：{{error}}",
              })}
            </p>
          )}
        </div>

        <div className="space-y-2">
          <FormLabel className="text-xs text-fg-2">
            {t("providerForm.localProxyBodyOverrides", {
              defaultValue: "Body 覆盖",
            })}
          </FormLabel>
          <Textarea
            value={bodyJson}
            onChange={(event) => onBodyJsonChange(event.target.value)}
            placeholder={'{\n  "temperature": 0.2\n}'}
            className="min-h-[132px] resize-y font-mono text-xs"
            aria-invalid={Boolean(bodyError)}
          />
          {bodyError && (
            <p className="text-xs text-destructive">
              {t("providerForm.localProxyBodyOverridesInvalidDetail", {
                error: bodyError,
                defaultValue: "Body 覆盖格式错误：{{error}}",
              })}
            </p>
          )}
        </div>
      </div>

      <div className="space-y-2">
        <FormLabel className="text-xs text-fg-2">
          {t("providerForm.localProxyBodyRewrites", {
            defaultValue: "Body 重写规则",
          })}
        </FormLabel>
        <p className="text-xs text-fg-2">
          {t("providerForm.localProxyBodyRewritesHint", {
            defaultValue:
              "按路径匹配请求体中的字符串值并条件替换，例如把 messages 内的 system 角色改为 user。",
          })}
        </p>
        <Textarea
          value={rewritesJson}
          onChange={(event) => onRewritesJsonChange(event.target.value)}
          placeholder={'{\n  "messages[*].role": { "system": "user" }\n}'}
          className="min-h-[96px] resize-y font-mono text-xs"
          aria-invalid={Boolean(rewritesError)}
        />
        {rewritesError && (
          <p className="text-xs text-destructive">
            {t("providerForm.localProxyBodyRewritesInvalidDetail", {
              error: rewritesError,
              defaultValue: "Body 重写规则格式错误：{{error}}",
            })}
          </p>
        )}
      </div>
    </div>
  );
}
