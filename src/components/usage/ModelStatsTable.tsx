import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useModelStats } from "@/lib/query/usage";
import { TablePagination, useClientPagination } from "./TablePagination";
import { cn } from "@/lib/utils";
import {
  fmtInt,
  fmtUsd,
  formatTokensCompact,
  formatTokensPerSecond,
  getAggregateTokensPerSecond,
  getLocaleFromLanguage,
  getResolvedLang,
} from "./format";
import { usageTable } from "./usageTable";
import type { UsageRangeSelection } from "@/types/usage";

interface ModelStatsTableProps {
  range: UsageRangeSelection;
  appType?: string;
  providerName?: string;
  model?: string;
  refreshIntervalMs: number;
}

export function ModelStatsTable({
  range,
  appType,
  providerName,
  model,
  refreshIntervalMs,
}: ModelStatsTableProps) {
  const { t, i18n } = useTranslation();
  const locale = getLocaleFromLanguage(getResolvedLang(i18n));
  const { data: stats, isLoading } = useModelStats(
    range,
    { appType, providerName, model },
    {
      refetchInterval: refreshIntervalMs > 0 ? refreshIntervalMs : false,
    },
  );

  const rows = useMemo(
    () => [...(stats ?? [])].sort((a, b) => b.requestCount - a.requestCount),
    [stats],
  );
  const pagination = useClientPagination(
    rows,
    JSON.stringify([range, appType, providerName, model]),
  );

  if (isLoading) {
    return <div className={usageTable.skeleton} />;
  }

  return (
    <div className="flex flex-col">
      <div className={usageTable.scroller}>
        <table
          className={cn(usageTable.table, "min-w-[640px]")}
          aria-label={t("usage.modelStats")}
        >
          <thead>
            <tr className={usageTable.headRow}>
              <th className={usageTable.th}>{t("usage.model")}</th>
              <th className={usageTable.thEnd}>{t("usage.requests")}</th>
              <th className={usageTable.thEnd}>{t("usage.tokens")}</th>
              <th className={usageTable.thEnd}>{t("usage.totalCost")}</th>
              <th className={usageTable.thEnd}>{t("usage.avgCost")}</th>
              <th className={usageTable.thEnd}>{t("usage.speed")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td colSpan={6} className={usageTable.empty}>
                  {t("usage.noData")}
                </td>
              </tr>
            ) : (
              pagination.pageRows.map((stat) => {
                // 一行模型的汇总速度：Σ输出 ÷ Σ生成时间（与供应商表同口径）
                const speed = formatTokensPerSecond(
                  getAggregateTokensPerSecond(
                    stat.streamOutputTokens,
                    stat.streamGenMs,
                  ),
                );
                return (
                  <tr key={stat.model} className={usageTable.row}>
                    <td className={cn(usageTable.td, usageTable.mono)}>
                      <span
                        className="block max-w-[320px] truncate"
                        title={stat.model}
                      >
                        {stat.model}
                      </span>
                    </td>
                    <td className={usageTable.tdEnd}>
                      {fmtInt(stat.requestCount, locale)}
                    </td>
                    <td
                      className={usageTable.tdEnd}
                      title={fmtInt(stat.totalTokens, locale)}
                    >
                      {formatTokensCompact(stat.totalTokens, locale)}
                    </td>
                    <td
                      className={cn(usageTable.tdEnd, "font-medium")}
                      title={fmtUsd(stat.totalCost, 6)}
                    >
                      {fmtUsd(stat.totalCost, 2)}
                    </td>
                    <td
                      className={cn(usageTable.tdEnd, "text-fg-2")}
                      title={fmtUsd(stat.avgCostPerRequest, 6)}
                    >
                      {fmtUsd(stat.avgCostPerRequest, 4)}
                    </td>
                    <td
                      className={cn(
                        usageTable.tdEnd,
                        speed == null && usageTable.muted,
                      )}
                    >
                      {speed == null ? (
                        "—"
                      ) : (
                        <>
                          {speed}
                          <span className="ms-0.5 text-badge font-normal text-fg-3">
                            tok/s
                          </span>
                        </>
                      )}
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
      <TablePagination
        page={pagination.page}
        totalPages={pagination.totalPages}
        total={pagination.total}
        onPageChange={pagination.setPage}
      />
    </div>
  );
}
