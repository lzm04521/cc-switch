import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SettingsFormState } from "@/hooks/useSettings";
import {
  AppWindow,
  MonitorUp,
  Power,
  EyeOff,
  CircleDot,
  RectangleVertical,
} from "lucide-react";
import { ToggleRow } from "@/components/ui/toggle-row";
import { Input } from "@/components/ui/input";
import { AnimatePresence, motion } from "framer-motion";
import { isLinux } from "@/lib/platform";

interface WindowSettingsProps {
  settings: SettingsFormState;
  onChange: (updates: Partial<SettingsFormState>) => void;
}

/** 弹窗尺寸合法区间（与后端 floating_ball.rs 常量一致，提交前再由后端 clamp 兜底） */
const PANEL_WIDTH_MIN = 240;
const PANEL_WIDTH_MAX = 480;
const PANEL_HEIGHT_MIN = 320;
const PANEL_HEIGHT_MAX = 800;
const PANEL_WIDTH_DEFAULT = 300;
const PANEL_HEIGHT_DEFAULT = 480;

function clampPanelInput(
  raw: string,
  min: number,
  max: number,
  fallback: number,
): number {
  const parsed = Number.parseFloat(raw);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(max, Math.max(min, Math.round(parsed)));
}

/**
 * 悬浮球弹窗尺寸输入（宽 × 高，逻辑像素）：本地编辑态 + 失焦/回车提交，
 * 避免逐键触发整份设置的自动保存；提交走 handleAutoSave，面板可见时后端即时重摆。
 */
function BallPanelSizeRow({
  floatingBall,
  onApply,
}: {
  floatingBall: SettingsFormState["floatingBall"];
  onApply: (value: NonNullable<SettingsFormState["floatingBall"]>) => void;
}) {
  const { t } = useTranslation();
  const appliedWidth = floatingBall?.panelWidth ?? PANEL_WIDTH_DEFAULT;
  const appliedHeight = floatingBall?.panelHeight ?? PANEL_HEIGHT_DEFAULT;
  const [width, setWidth] = useState(String(appliedWidth));
  const [height, setHeight] = useState(String(appliedHeight));

  // 外部设置变化（重置表单 / 保存回显）时同步编辑态；输入中途依赖不变不触发
  useEffect(() => {
    setWidth(String(appliedWidth));
  }, [appliedWidth]);
  useEffect(() => {
    setHeight(String(appliedHeight));
  }, [appliedHeight]);

  const apply = () => {
    const w = clampPanelInput(
      width,
      PANEL_WIDTH_MIN,
      PANEL_WIDTH_MAX,
      PANEL_WIDTH_DEFAULT,
    );
    const h = clampPanelInput(
      height,
      PANEL_HEIGHT_MIN,
      PANEL_HEIGHT_MAX,
      PANEL_HEIGHT_DEFAULT,
    );
    setWidth(String(w));
    setHeight(String(h));
    if (w === appliedWidth && h === appliedHeight) return;
    onApply({ ...floatingBall, panelWidth: w, panelHeight: h });
  };

  const commitOnEnter = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") e.currentTarget.blur();
  };

  return (
    <div className="flex items-center gap-2 py-1 pl-6">
      <RectangleVertical className="h-4 w-4 text-blue-500 shrink-0" />
      <span className="text-sm">{t("settings.floatingBallPanelSize")}</span>
      <Input
        type="number"
        min={PANEL_WIDTH_MIN}
        max={PANEL_WIDTH_MAX}
        value={width}
        onChange={(e) => setWidth(e.target.value)}
        onBlur={apply}
        onKeyDown={commitOnEnter}
        className="h-8 w-20"
        aria-label={t("settings.floatingBallPanelWidth")}
      />
      <span className="text-muted-foreground">×</span>
      <Input
        type="number"
        min={PANEL_HEIGHT_MIN}
        max={PANEL_HEIGHT_MAX}
        value={height}
        onChange={(e) => setHeight(e.target.value)}
        onBlur={apply}
        onKeyDown={commitOnEnter}
        className="h-8 w-20"
        aria-label={t("settings.floatingBallPanelHeight")}
      />
      <span className="text-xs text-muted-foreground">
        {t("settings.floatingBallPanelSizeHint")}
      </span>
    </div>
  );
}

export function WindowSettings({ settings, onChange }: WindowSettingsProps) {
  const { t } = useTranslation();

  return (
    <section className="space-y-4">
      <div className="flex items-center gap-2 pb-2 border-b border-border/40">
        <AppWindow className="h-4 w-4 text-primary" />
        <h3 className="text-sm font-medium">{t("settings.windowBehavior")}</h3>
      </div>

      <div className="space-y-3">
        <ToggleRow
          icon={<Power className="h-4 w-4 text-orange-500" />}
          title={t("settings.launchOnStartup")}
          description={t("settings.launchOnStartupDescription")}
          checked={!!settings.launchOnStartup}
          onCheckedChange={(value) => onChange({ launchOnStartup: value })}
        />

        <AnimatePresence initial={false}>
          {settings.launchOnStartup && (
            <motion.div
              key="silent-startup"
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 10 }}
              transition={{ duration: 0.3 }}
            >
              <ToggleRow
                icon={<EyeOff className="h-4 w-4 text-green-500" />}
                title={t("settings.silentStartup")}
                description={t("settings.silentStartupDescription")}
                checked={!!settings.silentStartup}
                onCheckedChange={(value) => onChange({ silentStartup: value })}
              />
            </motion.div>
          )}
        </AnimatePresence>

        <ToggleRow
          icon={<MonitorUp className="h-4 w-4 text-purple-500" />}
          title={t("settings.enableClaudePluginIntegration")}
          description={t("settings.enableClaudePluginIntegrationDescription")}
          checked={!!settings.enableClaudePluginIntegration}
          onCheckedChange={(value) =>
            onChange({ enableClaudePluginIntegration: value })
          }
        />

        <ToggleRow
          icon={<MonitorUp className="h-4 w-4 text-cyan-500" />}
          title={t("settings.skipClaudeOnboarding")}
          description={t("settings.skipClaudeOnboardingDescription")}
          checked={!!settings.skipClaudeOnboarding}
          onCheckedChange={(value) => onChange({ skipClaudeOnboarding: value })}
        />

        <ToggleRow
          icon={<AppWindow className="h-4 w-4 text-blue-500" />}
          title={t("settings.minimizeToTray")}
          description={t("settings.minimizeToTrayDescription")}
          checked={settings.minimizeToTrayOnClose}
          onCheckedChange={(value) =>
            onChange({ minimizeToTrayOnClose: value })
          }
        />

        <ToggleRow
          icon={<CircleDot className="h-4 w-4 text-blue-500" />}
          title={t("settings.floatingBall")}
          description={t("settings.floatingBallDescription")}
          checked={settings.floatingBall?.enabled ?? true}
          onCheckedChange={(value) =>
            // 展开完整对象：仅传 enabled 会让后端 serde 缺省值把弹窗尺寸重置回默认
            onChange({
              floatingBall: { ...settings.floatingBall, enabled: value },
            })
          }
        />

        {settings.floatingBall?.enabled && (
          <BallPanelSizeRow
            floatingBall={settings.floatingBall}
            onApply={(value) => onChange({ floatingBall: value })}
          />
        )}

        {isLinux() && (
          <ToggleRow
            icon={<AppWindow className="h-4 w-4 text-amber-500" />}
            title={t("settings.useAppWindowControls")}
            description={t("settings.useAppWindowControlsDescription")}
            checked={!!settings.useAppWindowControls}
            onCheckedChange={(value) =>
              onChange({ useAppWindowControls: value })
            }
          />
        )}
      </div>
    </section>
  );
}
