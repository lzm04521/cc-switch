import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import UsageFooter from "@/components/UsageFooter";
import { ProviderCard } from "@/components/providers/ProviderCard";
import { usageApi, type AppId } from "@/lib/api";
import type { Provider } from "@/types";

vi.mock("@/components/ProviderIcon", () => ({ ProviderIcon: () => null }));
vi.mock("@/components/quota/useQuotaDisplay", () => ({
  useQuotaDisplay: () => "left",
}));
vi.mock("@/lib/query/failover", () => ({
  useProviderHealth: () => ({ data: undefined }),
}));
// fork：UsageFooter 的后台轮询钳制读全局开关，mock 开启使钳制分支可测
vi.mock("@/lib/query/queries", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/query/queries")>()),
  useSettingsQuery: () => ({
    data: { autoRefreshAllProvidersUsage: true },
  }),
}));

const advance = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

describe.each(["footer", "collapsed card"] as const)(
  "%s usage polling",
  (view) => {
    let client: QueryClient;
    let query: MockInstance<typeof usageApi.query>;

    beforeEach(() => {
      vi.useFakeTimers();
      client = new QueryClient({
        defaultOptions: { queries: { retry: false } },
      });
      query = vi.spyOn(usageApi, "query").mockResolvedValue({
        success: true,
        data: [
          { planName: "Plan A", remaining: 10, unit: "USD" },
          { planName: "Plan B", remaining: 20, unit: "USD" },
        ],
      });
    });

    afterEach(() => {
      cleanup();
      client.clear();
      vi.restoreAllMocks();
      vi.useRealTimers();
    });

    async function renderUsage({
      appId = "claude",
      isCurrent = false,
      isInConfig = false,
      interval = 1,
      enabled = true,
    }: {
      appId?: AppId;
      isCurrent?: boolean;
      isInConfig?: boolean;
      interval?: number | null;
      enabled?: boolean;
    } = {}) {
      const provider: Provider = {
        id: "usage-provider",
        name: "Usage provider",
        category: "custom",
        settingsConfig: {
          env: { ANTHROPIC_BASE_URL: "https://example.com" },
        },
        meta: {
          usage_script: {
            enabled,
            language: "javascript",
            code: "",
            autoQueryInterval: interval ?? undefined,
          },
        },
      };

      render(
        <QueryClientProvider client={client}>
          {view === "footer" ? (
            <UsageFooter
              provider={provider}
              providerId={provider.id}
              appId={appId}
              usageEnabled={enabled}
              isCurrent={isCurrent}
              isInConfig={isInConfig}
            />
          ) : (
            <ProviderCard
              provider={provider}
              appId={appId}
              isCurrent={isCurrent}
              presentation={{ chips: [], buttons: [] }}
              onEdit={vi.fn()}
              onDelete={vi.fn()}
              onConfigureUsage={vi.fn()}
              onOpenWebsite={vi.fn()}
            />
          )}
        </QueryClientProvider>,
      );
      await advance(1);
      if (view === "collapsed card" && enabled) {
        // With the footer unmounted, the card must keep its own query polling.
        fireEvent.click(screen.getByRole("button", { name: "收起" }));
      }
    }

    it.each([
      { appId: "claude" as const, isCurrent: true, isInConfig: true },
      { appId: "opencode" as const, isCurrent: true, isInConfig: true },
    ])("refreshes every minute for the active provider (%j)", async (options) => {
      await renderUsage(options);
      expect(query).toHaveBeenCalledTimes(1);
      expect(query).toHaveBeenCalledWith("usage-provider", options.appId);

      await advance(60_000);
      expect(query).toHaveBeenCalledTimes(2);
      await advance(60_000);
      expect(query).toHaveBeenCalledTimes(3);
    });

    it.each([
      { appId: "claude" as const, isCurrent: false },
      { appId: "opencode" as const, isCurrent: false },
    ])(
      "clamps the background interval to 5 minutes for inactive providers (%j)",
      async (options) => {
        // fork：全局开关开启后非当前卡也刷新，但间隔至少 5 分钟
        await renderUsage(options);
        expect(query).toHaveBeenCalledTimes(1);

        await advance(60_000);
        expect(query).toHaveBeenCalledTimes(1);
        await advance(4 * 60_000);
        expect(query).toHaveBeenCalledTimes(2);
      },
    );

    it.each([0, null])(
      "does not poll with a zero or unset interval (%s)",
      async (interval) => {
        await renderUsage({ interval });
        expect(query).toHaveBeenCalledTimes(1);
        await advance(180_000);
        expect(query).toHaveBeenCalledTimes(1);
      },
    );

    it("does not query when usage is disabled", async () => {
      await renderUsage({ enabled: false });
      await advance(180_000);
      expect(query).not.toHaveBeenCalled();
    });
  },
);
