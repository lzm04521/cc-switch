import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import {
  DndContext,
  DragOverlay,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import {
  arrayMove,
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Check, ExternalLink, Zap } from "lucide-react";
import { toast } from "sonner";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ProviderIcon } from "@/components/ProviderIcon";
import {
  TierBadge,
  formatRelativeTime,
} from "@/components/SubscriptionQuotaFooter";
import { toQuotaTier } from "@/components/UsageFooter";
import { formatTokensShort, getResolvedLang } from "@/components/usage/format";
import type { UsageData } from "@/types";
import { providersApi } from "@/lib/api";
import type { AppId } from "@/lib/api";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import {
  FLOATING_BALL_SECTIONS_KEY,
  PROVIDER_USAGE_CACHE_KEY,
  floatingBallApi,
  useFloatingBallSections,
  useProviderUsageCache,
  type BallProviderInfo,
  type BallSection,
  type UsageCacheSnapshot,
} from "@/lib/api/floatingBall";
import { useUsageSummary, usageKeys } from "@/lib/query/usage";
import { cn } from "@/lib/utils";
import { applyProviderSwitchSideEffects } from "@/hooks/providerSwitchSideEffects";

/** 面板分组显示名（与 AppSwitcher 保持一致） */
export const APP_DISPLAY_NAME: Record<AppId, string> = {
  claude: "Claude Code",
  "claude-desktop": "Claude Desktop",
  codex: "Codex",
  gemini: "Gemini",
  grokbuild: "Grok Build",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  hermes: "Hermes",
  zcode: "ZCode",
};

/** 余额/次数类单条展示：剩余值 + 单位（不足 10% 变橙、失效变红，梯度同主界面） */
function BalanceItem({ data }: { data: UsageData }) {
  const { t } = useTranslation();
  const isExpired = data.isValid === false;
  const low =
    data.remaining !== undefined &&
    data.remaining < (data.total ?? data.remaining) * 0.1;
  return (
    <span className="floating-ball-usage-item">
      💰 {t("usage.remaining")}
      <span
        className={cn(
          "font-semibold tabular-nums",
          isExpired
            ? "text-red-500 dark:text-red-400"
            : low
              ? "text-orange-500 dark:text-orange-400"
              : "text-green-600 dark:text-green-400",
        )}
      >
        {data.remaining !== undefined ? data.remaining.toFixed(2) : "—"}
      </span>
      {data.unit && <span>{data.unit}</span>}
      {isExpired && (
        <span className="text-red-500 dark:text-red-400">
          {data.invalidMessage || t("usage.invalid")}
        </span>
      )}
    </span>
  );
}

/**
 * 行下用量子行（横向单行，flex-wrap 兜底）：仅在缓存命中时渲染。
 * 数据取自后端 UsageCache（主窗口 / 托盘触发的查询写穿），面板不发网络请求：
 * - Token Plan / 订阅类（unit === "%"）：TierBadge（与主界面 inline 展示一致）
 * - 余额类：BalanceItem
 * - 失败：红字查询失败（不吞错）
 * 最右为查询相对时间（数据新鲜度）。
 */
function UsageSubRow({ snapshot }: { snapshot: UsageCacheSnapshot }) {
  const { t } = useTranslation();
  const { result, queriedAt } = snapshot;

  const list = result.data ?? [];
  // 成功但无数据：与主界面 UsageFooter 一致，整体不渲染
  if (result.success && list.length === 0) return null;

  return (
    <span className="floating-ball-row-usage">
      {!result.success ? (
        <span className="floating-ball-usage-fail">
          ⚠ {result.error || t("usage.queryFailed")}
        </span>
      ) : (
        list.map((d, i) =>
          d.unit === "%" ? (
            <TierBadge key={i} tier={toQuotaTier(d)} t={t} />
          ) : (
            <BalanceItem key={i} data={d} />
          ),
        )
      )}
      <span className="floating-ball-usage-time">
        {formatRelativeTime(queriedAt, Date.now(), t)}
      </span>
    </span>
  );
}

/**
 * 可拖拽排序的 provider 行（作为拖拽触发源）：
 * - 按下移动超过 8px → 进入拖拽，原行隐藏（opacity 0 占位），
 *   内容由 DragOverlay 渲染（脱离 scroll 容器，跟手零裁剪）
 * - 按下后未移动即松开 → 点击切换（与原有交互一致）
 */
function SortableProviderRow({
  provider,
  isCurrent,
  usage,
  onClick,
}: {
  provider: BallProviderInfo;
  isCurrent: boolean;
  /** 后端用量缓存命中项；未开启用量查询 / 从未查过的 provider 为 undefined */
  usage?: UsageCacheSnapshot;
  onClick: () => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: provider.id });
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
  };
  return (
    <button
      ref={setNodeRef}
      type="button"
      style={style}
      {...attributes}
      {...listeners}
      className={cn(
        "floating-ball-row",
        isCurrent && "is-current",
        usage && "has-usage",
        // 拖拽中的原行：占位隐藏（内容在 DragOverlay 中显示）
        isDragging && "floating-ball-row-dragging",
      )}
      onClick={onClick}
    >
      <ProviderIcon
        icon={provider.icon ?? undefined}
        name={provider.name}
        size={18}
      />
      <span className="floating-ball-row-name">{provider.name}</span>
      {isCurrent && <Check className="floating-ball-check" size={14} />}
      {usage && <UsageSubRow snapshot={usage} />}
    </button>
  );
}

/**
 * 今日总Token小块（footer 内、「打开主界面」上方）：当天全部 app 的真实总消耗
 * token，口径与主窗口用量页 Hero 一致（input + output + cache_creation +
 * cache_read）；主色浅底卡片 + 大号数值突出显示，与按钮之间由
 * floating-ball-footer-divider 横线分隔
 */
function TodayUsageBlock() {
  const { t, i18n } = useTranslation();
  const { data: summary } = useUsageSummary({ preset: "today" });
  const realTotal = summary?.realTotalTokens ?? 0;
  return (
    <div className="floating-ball-usage">
      <span className="floating-ball-usage-label">
        <Zap size={13} className="floating-ball-usage-icon" />
        {t("floatingBall.todayUsage", { defaultValue: "今日总Token" })}
      </span>
      <span className="floating-ball-usage-value">
        {formatTokensShort(realTotal, getResolvedLang(i18n))}
      </span>
    </div>
  );
}

/**
 * 悬浮球弹出面板（panel 窗口内渲染）：
 * - 按 app 分组的 provider 列表，当前项勾选高亮
 * - 点击行 → switch_provider，成功后收起面板，失败内联提示可重试
 * - 分组内拖拽调整顺序（DragOverlay 跟随，原行占位隐藏）→
 *   复用 updateSortOrder 写入 sort_index（全局同步）
 * - Esc / 失焦（后端判断焦点）收起；footer 提供「打开主界面」
 */
export function PanelList() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: sections } = useFloatingBallSections();
  const { data: usageSnapshots } = useProviderUsageCache();
  const [activeId, setActiveId] = useState<string | null>(null);
  // 拖放后的短暂落位窗口：禁用行过渡，避免"让位归位 + 数据重排滑动"叠加成摆动
  const [isSettling, setIsSettling] = useState(false);

  // 用量缓存快照按 app:provider 建索引，行渲染 O(1) 匹配
  const usageByProvider = useMemo(() => {
    const map = new Map<string, UsageCacheSnapshot>();
    usageSnapshots?.forEach((s) =>
      map.set(`${s.appType}:${s.providerId}`, s),
    );
    return map;
  }, [usageSnapshots]);

  // 主窗口 / 托盘触发的用量查询写穿缓存后会广播该事件；面板打开期间实时同步
  useTauriEvent("usage-cache-updated", () => {
    void queryClient.invalidateQueries({ queryKey: PROVIDER_USAGE_CACHE_KEY });
  });

  // provider 切换事件 → 刷新分组（与主窗口同事件源）
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void providersApi
      .onSwitched(() => {
        void queryClient.invalidateQueries({
          queryKey: FLOATING_BALL_SECTIONS_KEY,
        });
      })
      .then((off) => {
        if (disposed) off();
        else cleanup = off;
      });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [queryClient]);

  // Esc 收起面板
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        void floatingBallApi.hidePanel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 获得焦点（每次打开面板）刷新分组；失焦收起（后端判断新焦点是否落在
  // 球上，点球关面板不重复处理）。focus 刷新兜底事件时序竞态：主窗口切换
  // 供应商时面板窗口若错过 provider-switched 事件，下次打开仍能拿到新数据。
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    const appWindow = getCurrentWindow();
    void appWindow
      .onFocusChanged(({ payload: focused }) => {
        if (focused) {
          void queryClient.invalidateQueries({
            queryKey: FLOATING_BALL_SECTIONS_KEY,
          });
          // 今日用量随面板每次打开刷新（另有默认 30s 轮询兜底）
          void queryClient.invalidateQueries({ queryKey: usageKeys.all });
          // 用量缓存快照随面板打开重取（主窗口在此期间的查询也会经事件同步）
          void queryClient.invalidateQueries({
            queryKey: PROVIDER_USAGE_CACHE_KEY,
          });
        } else {
          void floatingBallApi.onPanelBlur();
        }
      })
      .then((off) => {
        if (disposed) off();
        else cleanup = off;
      });
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [queryClient]);

  const handleSwitch = async (appType: AppId, providerId: string) => {
    try {
      const result = await providersApi.switch(providerId, appType);
      // 补齐主窗口 switchProvider 的副作用链（插件联动/重启提示/代理警告）
      await applyProviderSwitchSideEffects(appType, providerId, {
        t,
        switchResult: result,
      });
      await floatingBallApi.hidePanel();
    } catch {
      toast.error(
        t("floatingBall.switchFailed", { defaultValue: "切换失败，请重试" }),
      );
    }
  };

  // 拖拽排序：同一分组内重排 → 写入 sort_index（与主界面 useDragSort 一致）
  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id || !sections) return;

    // 定位 active / over 所属分组；跨分组拖拽直接忽略
    const activeSection = sections.find((s) =>
      s.providers.some((p) => p.id === active.id),
    );
    const overSection = sections.find((s) =>
      s.providers.some((p) => p.id === over.id),
    );
    if (!activeSection || activeSection !== overSection) return;

    const oldIndex = activeSection.providers.findIndex(
      (p) => p.id === active.id,
    );
    const newIndex = activeSection.providers.findIndex((p) => p.id === over.id);
    if (oldIndex === -1 || newIndex === -1) return;

    const reordered = arrayMove(activeSection.providers, oldIndex, newIndex);
    const updates = reordered.map((provider, index) => ({
      id: provider.id,
      sortIndex: index,
    }));

    try {
      await providersApi.updateSortOrder(updates, activeSection.appType);
      await queryClient.invalidateQueries({
        queryKey: FLOATING_BALL_SECTIONS_KEY,
      });
      // 同步托盘菜单排序（失败不影响主操作）
      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error("Failed to update tray menu after sort", trayError);
      }
    } catch (error) {
      console.error("Failed to update provider sort order", error);
      toast.error(
        t("provider.sortUpdateFailed", { defaultValue: "排序更新失败" }),
      );
    }
  };

  const handleDragStart = (event: DragStartEvent) => {
    setActiveId(event.active.id as string);
  };

  const handleDragEndEvent = (event: DragEndEvent) => {
    setActiveId(null);
    // 落位窗口内禁行过渡，让行直接归位（数据刷新后再开启）
    setIsSettling(true);
    window.setTimeout(() => setIsSettling(false), 350);
    void handleDragEnd(event);
  };

  const handleDragCancel = () => {
    setActiveId(null);
    setIsSettling(true);
    window.setTimeout(() => setIsSettling(false), 350);
  };

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
  );

  // 每个分组独立 DndContext：verticalListSortingStrategy 只对该分组内的
  // 行计算让位偏移，拖拽 claude 组时 codex 等其他分组的行完全不受影响
  const renderSection = (section: BallSection) => {
    const sectionActiveProvider = section.providers.find(
      (p) => p.id === activeId,
    );
    const activeUsage = sectionActiveProvider
      ? usageByProvider.get(`${section.appType}:${sectionActiveProvider.id}`)
      : undefined;
    return (
      <div key={section.appType} className="floating-ball-section">
        <div className="floating-ball-section-header">
          {APP_DISPLAY_NAME[section.appType]}
        </div>
        {section.providers.length === 0 ? (
          <div className="floating-ball-empty">
            {t("floatingBall.empty", { defaultValue: "（无供应商）" })}
          </div>
        ) : (
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragStart={handleDragStart}
            onDragEnd={handleDragEndEvent}
            onDragCancel={handleDragCancel}
          >
            <SortableContext
              items={section.providers.map((p) => p.id)}
              strategy={verticalListSortingStrategy}
            >
              {section.providers.map((provider) => (
                <SortableProviderRow
                  key={provider.id}
                  provider={provider}
                  isCurrent={provider.id === section.currentProviderId}
                  usage={usageByProvider.get(
                    `${section.appType}:${provider.id}`,
                  )}
                  onClick={() =>
                    void handleSwitch(section.appType, provider.id)
                  }
                />
              ))}
            </SortableContext>
            <DragOverlay dropAnimation={null}>
              {sectionActiveProvider ? (
                <div
                  className={cn(
                    "floating-ball-row",
                    "floating-ball-row-overlay",
                    sectionActiveProvider.id === section.currentProviderId &&
                      "is-current",
                    activeUsage && "has-usage",
                  )}
                >
                  <ProviderIcon
                    icon={sectionActiveProvider.icon ?? undefined}
                    name={sectionActiveProvider.name}
                    size={18}
                  />
                  <span className="floating-ball-row-name">
                    {sectionActiveProvider.name}
                  </span>
                  {sectionActiveProvider.id === section.currentProviderId && (
                    <Check className="floating-ball-check" size={14} />
                  )}
                  {activeUsage && <UsageSubRow snapshot={activeUsage} />}
                </div>
              ) : null}
            </DragOverlay>
          </DndContext>
        )}
      </div>
    );
  };

  return (
    <div
      className={cn(
        "floating-ball-panel",
        activeId != null && "is-dragging",
        isSettling && "is-settling",
      )}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="floating-ball-panel-scroll">
        {sections?.map(renderSection)}
      </div>
      <div className="floating-ball-footer">
        <TodayUsageBlock />
        <div className="floating-ball-footer-divider" />
        <button
          type="button"
          className="floating-ball-footer-btn"
          onClick={() => void floatingBallApi.showMainWindow()}
        >
          <ExternalLink size={14} />
          {t("floatingBall.openMainWindow", { defaultValue: "打开主界面" })}
        </button>
      </div>
    </div>
  );
}
