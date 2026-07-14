import type { ReactNode } from "react";
import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDeleteProviderMutation } from "@/lib/query/mutations";

const mocks = vi.hoisted(() => ({
  deleteProvider: vi.fn(),
  updateTrayMenu: vi.fn(),
  invalidateAggregationSources: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  providersApi: {
    delete: (...args: unknown[]) => mocks.deleteProvider(...args),
    updateTrayMenu: (...args: unknown[]) => mocks.updateTrayMenu(...args),
  },
  sessionsApi: {},
  settingsApi: {},
}));

vi.mock("@/lib/query/aggregation", () => ({
  invalidateAggregationSources: (...args: unknown[]) =>
    mocks.invalidateAggregationSources(...args),
}));

vi.mock("@/hooks/useHermes", () => ({
  invalidateHermesProviderCaches: vi.fn(),
}));

vi.mock("@/hooks/useOpenClaw", () => ({
  openclawKeys: {
    health: ["openclaw", "health"],
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? _key,
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => mocks.toastSuccess(...args),
    error: vi.fn(),
  },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  return { wrapper, queryClient, invalidateSpy };
}

beforeEach(() => {
  mocks.deleteProvider.mockReset().mockResolvedValue(undefined);
  mocks.updateTrayMenu.mockReset().mockResolvedValue(undefined);
  mocks.invalidateAggregationSources.mockReset();
  mocks.toastSuccess.mockReset();
});

describe("useDeleteProviderMutation", () => {
  it("invalidates aggregation sources for the current app after deletion", async () => {
    const { wrapper, queryClient, invalidateSpy } = createWrapper();
    const { result } = renderHook(() => useDeleteProviderMutation("claude"), {
      wrapper,
    });

    await act(async () => {
      await result.current.mutateAsync("provider-1");
    });

    expect(mocks.deleteProvider).toHaveBeenCalledWith("provider-1", "claude");
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: ["providers", "claude"],
    });
    expect(mocks.invalidateAggregationSources).toHaveBeenCalledTimes(1);
    expect(mocks.invalidateAggregationSources).toHaveBeenCalledWith(
      queryClient,
      "claude",
    );
    expect(mocks.updateTrayMenu).toHaveBeenCalledTimes(1);
    expect(mocks.toastSuccess).toHaveBeenCalledTimes(1);
  });
});
