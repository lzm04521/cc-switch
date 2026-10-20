import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AlertTriangle,
  ExternalLink,
  Feather,
  Gauge,
  Settings,
  Zap,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
// tp-scroll 滚动条样式（其 body.tray-panel 作用域选择器对本窗口无副作用）
import "@/tray-panel/tray-panel.css";
// 悬浮球面板复用官方托盘面板的行 / 子面板组件与样式常量（fork：托盘内容托管）
import {
  AppRow,
  FOCUS_RING,
  ROW_HOVER,
  SubPanel,
} from "@/tray-panel/TrayPanel";
import { UsageTab } from "@/tray-panel/UsageTab";
import { CHANGED_EVENT, trayPanelApi } from "@/tray-panel/api";
import { formatTokensShort, getResolvedLang } from "@/components/usage/format";
import { providersApi } from "@/lib/api";
import { floatingBallApi, type BallPanelLayout } from "@/lib/api/floatingBall";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { useUsageSummary, usageKeys } from "@/lib/query/usage";
import { cn } from "@/lib/utils";

type Tab = "config" | "usage";
/** 与托盘面板共享页签记忆（WebView2 同源 localStorage） */
const TAB_KEY = "cc-switch-tray-tab";
const BALL_TRAY_STATE_KEY = ["ball-tray-panel-state"] as const;
/** 悬停展开 / 移出收起覆盖层的节奏，沿用托盘面板 */
const HOVER_OPEN_MS = 50;
const HOVER_CLOSE_MS = 160;
/** 设置尺寸变化触发的面板重摆事件（后端 BALL_PANEL_RELAYOUT_EVENT） */
const BALL_PANEL_RELAYOUT_EVENT = "ball-panel-relayout";
/** 布局拉取失败/为空时的兜底：覆盖层模式（忽略尺寸字段，非 split 均走旧结构） */
const FALLBACK_OVERLAY_LAYOUT: BallPanelLayout = {
  layout: "overlay",
  side: "right",
  mainWidth: 300,
  subWidth: 300,
  gap: 8,
};

function readTab(): Tab {
  try {
    return window.localStorage.getItem(TAB_KEY) === "usage"
      ? "usage"
      : "config";
  } catch {
    return "config";
  }
}

/**
 * 今日总Token小块（底部保留区内、「打开主界面」上方）：当天全部 app 的真实总消耗
 * token，口径与主窗口用量页 Hero 一致（input + output + cache_creation +
 * cache_read）；今日有可计算的代理直录流式请求时，第二行追加今日加权平均输出速度
 * （avgTokensPerSecond 为 null = 无路由数据 / 数据未加载，均不渲染该行）。
 */
function TodayUsageBlock() {
  const { t, i18n } = useTranslation();
  const { data: summary } = useUsageSummary({ preset: "today" });
  const realTotal = summary?.realTotalTokens ?? 0;
  const avgTps = summary?.avgTokensPerSecond ?? null;
  return (
    <div className="floating-ball-usage">
      <div className="floating-ball-usage-row">
        <span className="floating-ball-usage-label">
          <Zap size={13} className="floating-ball-usage-icon" />
          {t("floatingBall.todayUsage", { defaultValue: "今日总Token" })}
        </span>
        <span className="floating-ball-usage-value">
          {formatTokensShort(realTotal, getResolvedLang(i18n))}
        </span>
      </div>
      {avgTps != null && (
        <div className="floating-ball-usage-row">
          <span className="floating-ball-usage-label">
            <Gauge size={13} className="floating-ball-usage-icon" />
            {t("floatingBall.todaySpeed", { defaultValue: "今日输出速度" })}
          </span>
          <span className="floating-ball-usage-value floating-ball-usage-value-sub">
            {avgTps.toFixed(1)} t/s
          </span>
        </div>
      )}
    </div>
  );
}

/**
 * 悬浮球弹出面板（panel 窗口内渲染，fork：内容托管官方托盘面板）：
 * - 主体 = 托盘面板主卡片：头部（CC Switch + 设置入口）、配置 / 用量页签、
 *   问题提醒区、应用行（当前供应商 / 模式 / 额度）、footer 轻量模式行；
 *   「打开主界面 / 退出」按钮不搬（前者在底部保留区、后者按用户要求不提供）
 * - hover 应用行 → 子面板：Windows 固定画布双栏（split），子卡从主卡左/右侧
 *   弹出（side 跟随球位置翻转），透明预留区由后端裁剪穿透；其他平台沿用
 *   窗口内覆盖层（overlay，锚到行下方限高滚动）。点击行可切换展开态；
 *   原「供应商拖拽排序」已随分组列表退役
 * - 底部保留区 = 今日用量统计 +「打开主界面」（原面板下半部分）
 * - 数据与文案复用 tray_panel_state / tray_panel_action（与托盘面板同源）；
 *   刷新时机：面板 focus（每次打开）、provider 切换事件、tray-panel-changed
 * - Esc：子面板开着先收子面板，否则收起面板；失焦由后端判断收起
 */
export function PanelList() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data: state } = useQuery({
    queryKey: BALL_TRAY_STATE_KEY,
    queryFn: () => trayPanelApi.state(),
  });

  const [tab, setTab] = useState<Tab>(readTab);
  /** 覆盖层当前展开的应用 id（行数据每次从最新 state 取，切换后勾选会移动） */
  const [openApp, setOpenApp] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  /** 面板每次打开递增：UsageTab 的 refreshKey */
  const [shownCount, setShownCount] = useState(0);
  /** 面板画布布局（面板打开时拉取）：split=Windows 固定画布双栏，overlay=覆盖层 */
  const [panelLayout, setPanelLayout] = useState<BallPanelLayout | null>(null);
  const split = panelLayout?.layout === "split";

  const rootRef = useRef<HTMLDivElement>(null);
  const overlayRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const hoverTimer = useRef<number | undefined>(undefined);
  const hideTimer = useRef<number | undefined>(undefined);

  // 托盘状态变化（切换、额度、问题区）后端会一并通知本窗口；面板打开期间实时同步
  useTauriEvent(CHANGED_EVENT, () => {
    void queryClient.invalidateQueries({ queryKey: BALL_TRAY_STATE_KEY });
  });

  // 设置尺寸变化 → 后端已重摆（子面板区域收敛回主卡）：重拉布局并收起子卡
  useTauriEvent(BALL_PANEL_RELAYOUT_EVENT, () => {
    void floatingBallApi
      .getPanelLayout()
      .then((layout) => setPanelLayout(layout ?? FALLBACK_OVERLAY_LAYOUT))
      .catch(() => setPanelLayout(FALLBACK_OVERLAY_LAYOUT));
    setOpenApp(null);
  });

  // provider 切换事件 → 刷新面板数据（与主窗口同事件源；切换后覆盖层勾选移动）
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void providersApi
      .onSwitched(() => {
        void queryClient.invalidateQueries({
          queryKey: BALL_TRAY_STATE_KEY,
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

  /** 子面板显隐统一出口：split 模式下同步扩/缩窗口命中区域（所有收起路径
   *  必经此处，防透明预留区漏缩回挡鼠标） */
  const applySub = useCallback(
    (appId: string | null) => {
      setOpenApp(appId);
      if (!split) return;
      if (appId) {
        // 先绘制子卡，双 rAF 后再扩原生命中区域，防透明底色一帧闪现（对齐托盘）
        requestAnimationFrame(() =>
          requestAnimationFrame(
            () => void floatingBallApi.setSubPanelVisible(true),
          ),
        );
      } else {
        void floatingBallApi.setSubPanelVisible(false);
      }
    },
    [split],
  );

  // 挂载时预拉一次布局（面板窗口常驻；真机此时面板未摆过多为 null，
  // 待首次打开 focus 再拉；测试环境无 focus 事件，靠这里拿到布局）
  useEffect(() => {
    void floatingBallApi
      .getPanelLayout()
      .then((layout) => {
        if (layout) setPanelLayout(layout);
      })
      .catch(() => {
        // 拉取失败不落地，等 focus 重拉
      });
  }, []);

  // Esc：覆盖层开着先收覆盖层，否则收起面板
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (openApp) {
        applySub(null);
        return;
      }
      void floatingBallApi.hidePanel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openApp, applySub]);

  // 获得焦点（每次打开面板）刷新数据并重置覆盖层；失焦收起（后端判断新焦点
  // 是否落在球上，点球关面板不重复处理）
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    const appWindow = getCurrentWindow();
    void appWindow
      .onFocusChanged(({ payload: focused }) => {
        if (focused) {
          void queryClient.invalidateQueries({
            queryKey: BALL_TRAY_STATE_KEY,
          });
          // 今日用量随面板每次打开刷新（另有默认 30s 轮询兜底）
          void queryClient.invalidateQueries({ queryKey: usageKeys.all });
          // 画布布局随打开拉取（side 跟随球位置翻转）
          void floatingBallApi
            .getPanelLayout()
            .then((layout) => setPanelLayout(layout ?? FALLBACK_OVERLAY_LAYOUT))
            .catch(() => setPanelLayout(FALLBACK_OVERLAY_LAYOUT));
          applySub(null);
          setShownCount((count) => count + 1);
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
  }, [queryClient, applySub]);

  const clearTimers = useCallback(() => {
    window.clearTimeout(hoverTimer.current);
    window.clearTimeout(hideTimer.current);
  }, []);

  const hoverApp = useCallback(
    (appId: string) => {
      clearTimers();
      hoverTimer.current = window.setTimeout(() => {
        applySub(appId);
      }, HOVER_OPEN_MS);
    },
    [clearTimers, applySub],
  );

  /** 移出应用行：稍等片刻再收（给鼠标滑进覆盖层留时间） */
  const scheduleHide = useCallback(() => {
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => {
      applySub(null);
    }, HOVER_CLOSE_MS);
  }, [applySub]);

  const openSub = useCallback(
    (appId: string | null) => {
      clearTimers();
      applySub(appId);
    },
    [clearTimers, applySub],
  );

  // 覆盖层锚到展开行的下方：内容或页签变化后重算位置，放不下时整体上移
  useEffect(() => {
    const root = rootRef.current;
    const overlay = overlayRef.current;
    if (!openApp || !root || !overlay) return;
    overlay.style.top = "0px";
    const rootTop = root.getBoundingClientRect().top;
    const row = root.querySelector<HTMLElement>(`[data-app-row="${openApp}"]`);
    const rowBottom = row
      ? row.getBoundingClientRect().bottom
      : rootTop + root.clientHeight / 2;
    const height = overlay.offsetHeight;
    let top = rowBottom - rootTop + 4;
    const maxTop = root.clientHeight - height - 8;
    if (top > maxTop) top = Math.max(8, maxTop);
    overlay.style.top = `${top}px`;
  }, [openApp, tab, state]);

  const changeTab = (next: Tab) => {
    setTab(next);
    applySub(null);
    setQuery("");
    try {
      window.localStorage.setItem(TAB_KEY, next);
    } catch {
      // 记不住也不影响使用
    }
  };

  const action = useCallback((id: string) => {
    void trayPanelApi.action(id);
  }, []);

  const texts = state?.texts;
  const apps = state?.apps ?? [];
  const openEntry =
    tab === "config"
      ? apps.find((app) => app.app === openApp && !app.addId)
      : undefined;

  if (!state || !texts || !panelLayout) {
    // 布局未就绪的短暂空壳：面板显示前必经后端定位（布局随之写入），
    // focus 拉取几 ms 内返回；拉取失败回落 overlay 不会停留在此
    return (
      <div
        className="floating-ball-panel"
        onContextMenu={(e) => e.preventDefault()}
      />
    );
  }

  // 主卡内容（头部/页签/列表/footer/底部保留区），split 与 overlay 两种外壳共用
  const mainCard = (
    <>
      {/* 头部：标题 + 设置入口 */}
      <div className="flex shrink-0 items-center px-4 pb-2 pt-3">
        <span className="text-[15px] font-semibold">CC Switch</span>
        <button
          type="button"
          title={texts.settings}
          aria-label={texts.settings}
          onClick={() => action("nav:settings:general")}
          className={cn(
            "ms-auto flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground",
            FOCUS_RING,
            ROW_HOVER,
          )}
        >
          <Settings className="h-4 w-4" />
        </button>
      </div>

      {/* 配置 / 用量页签（与托盘面板共享记忆） */}
      <div className="shrink-0 px-3 pb-2">
        <div
          role="tablist"
          className="flex rounded-lg bg-black/[0.06] p-0.5 dark:bg-white/[0.08]"
        >
          {(["config", "usage"] as const).map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={tab === value}
              onClick={() => changeTab(value)}
              className={cn(
                "flex-1 rounded-md py-1 text-xs font-medium transition-colors",
                FOCUS_RING,
                tab === value
                  ? "bg-white text-foreground shadow-sm dark:bg-white/[0.16]"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {value === "config" ? texts.configTab : texts.usageTab}
            </button>
          ))}
        </div>
      </div>

      <div className="tp-scroll min-h-0 flex-1">
        {tab === "config" ? (
          <div className="flex flex-col gap-0.5 px-1.5 pb-1.5">
            {state.problems.map((problem) => (
              <button
                key={problem.id}
                type="button"
                onClick={() => action(problem.id)}
                className={cn(
                  "flex items-start gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs text-amber-600 dark:text-amber-400",
                  FOCUS_RING,
                  ROW_HOVER,
                )}
              >
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>{problem.text}</span>
              </button>
            ))}
            {state.feedback && (
              <div className="px-2.5 py-1 text-[11.5px] text-muted-foreground">
                {state.feedback}
              </div>
            )}
            {apps.map((app) => (
              <AppRow
                key={app.app}
                app={app}
                texts={texts}
                active={openEntry?.app === app.app}
                side="right"
                onHover={() => hoverApp(app.app)}
                onLeave={scheduleHide}
                onOpen={() =>
                  app.addId
                    ? action(app.addId)
                    : openSub(openApp === app.app ? null : app.app)
                }
              />
            ))}
          </div>
        ) : (
          <UsageTab refreshKey={shownCount} visible={tab === "usage"} />
        )}
      </div>

      {/* footer：只保留轻量模式（打开主界面在底部保留区；退出按钮不提供） */}
      <div className="shrink-0 border-t border-black/[0.08] px-1.5 py-1.5 dark:border-white/[0.08]">
        <button
          type="button"
          role="switch"
          aria-checked={state.lightweight}
          onClick={() => action("lightweight_mode")}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[13px]",
            FOCUS_RING,
            ROW_HOVER,
          )}
        >
          <span className="flex h-5 w-5 shrink-0 items-center justify-center text-muted-foreground">
            <Feather className="h-4 w-4" />
          </span>
          <span className="flex-1">{texts.lightweightMode}</span>
          <span
            className={cn(
              "relative h-[18px] w-8 shrink-0 rounded-full transition-colors",
              state.lightweight
                ? "bg-blue-500"
                : "bg-black/15 dark:bg-white/20",
            )}
          >
            <span
              className={cn(
                "absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white shadow transition-[left]",
                state.lightweight ? "left-[16px]" : "left-[2px]",
              )}
            />
          </span>
        </button>
      </div>

      {/* 底部保留区（原面板下半部分） */}
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
    </>
  );

  /** 子面板内容：split 模式放旁边子卡，overlay 模式放窗口内覆盖层 */
  const subPanel = openEntry ? (
    <SubPanel
      key={openEntry.app}
      app={openEntry}
      texts={texts}
      query={query}
      onQuery={setQuery}
      onAction={action}
      searchRef={searchRef}
    />
  ) : null;

  // 固定画布双栏（Windows）：主卡与子卡绝对定位，画布透明预留区由后端
  // SetWindowRgn 裁剪（点击穿透）；子卡随 side 从主卡左/右侧弹出
  if (split) {
    const mainX =
      panelLayout.side === "right" ? 0 : panelLayout.subWidth + panelLayout.gap;
    const subX =
      panelLayout.side === "right"
        ? panelLayout.mainWidth + panelLayout.gap
        : 0;
    return (
      <div
        className="floating-ball-canvas"
        onContextMenu={(e) => e.preventDefault()}
      >
        <div
          ref={rootRef}
          className="floating-ball-panel absolute"
          style={{ left: mainX, top: 0, width: panelLayout.mainWidth }}
        >
          {mainCard}
        </div>
        {openEntry && (
          <div
            role="dialog"
            aria-label={openEntry.name}
            className="floating-ball-panel absolute"
            style={{ left: subX, top: 0, width: panelLayout.subWidth }}
            onMouseEnter={clearTimers}
            onMouseLeave={scheduleHide}
          >
            {subPanel}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      ref={rootRef}
      className="floating-ball-panel relative"
      onContextMenu={(e) => e.preventDefault()}
    >
      {mainCard}

      {/* hover 覆盖层子面板：复用托盘 SubPanel，窗口内浮动、锚到展开行 */}
      {openEntry && (
        <div
          ref={overlayRef}
          role="dialog"
          aria-label={openEntry.name}
          className="absolute inset-x-2 z-20 flex max-h-[70%] flex-col overflow-hidden rounded-xl border border-border bg-popover text-popover-foreground shadow-2xl"
          onMouseEnter={clearTimers}
          onMouseLeave={scheduleHide}
        >
          {subPanel}
        </div>
      )}
    </div>
  );
}
