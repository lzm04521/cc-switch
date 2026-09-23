import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";

interface FailoverPriorityBadgeProps {
  priority: number; // 1, 2, 3, ...
  className?: string;
}

/**
 * 故障转移优先级徽章
 * 显示供应商在故障转移队列中的优先级顺序
 */
export function FailoverPriorityBadge({
  priority,
  className,
}: FailoverPriorityBadgeProps) {
  const { t } = useTranslation();

  return (
    <div
      className={cn(
        // 徽章随异步数据条件显隐，淡入替代瞬间弹出（CSS 动画仅节点首次插入时播放）
        "inline-flex items-center px-1.5 py-0.5 rounded text-xs font-semibold animate-in fade-in-0 zoom-in-95 duration-200",
        "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
        className,
      )}
      title={t("failover.priority.tooltip", {
        priority,
        defaultValue: `故障转移优先级 ${priority}`,
      })}
    >
      P{priority}
    </div>
  );
}
