import type { ReactNode } from "react";
import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useExternalConfigBridge } from "@/hooks/useExternalConfigBridge";
import { createTestQueryClient } from "../utils/testQueryClient";

const invokeMock = vi.fn();

// 捕获注册的 external-config-changed 监听器，测试里手动触发。
let eventHandler: ((payload: unknown) => void) | undefined;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@/hooks/useTauriEvent", () => ({
  useTauriEvent: (_event: string, handler: (payload: unknown) => void) => {
    eventHandler = handler;
  },
}));

interface WrapperProps {
  children: ReactNode;
}

function createWrapper() {
  const queryClient = createTestQueryClient();
  const wrapper = ({ children }: WrapperProps) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return { wrapper, queryClient };
}

describe("useExternalConfigBridge", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    eventHandler = undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_external_config_status") {
        return Promise.resolve([]);
      }
      return Promise.resolve(null);
    });
  });

  it("hydrates conflict queue from existing conflicts on mount", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_external_config_status") {
        return Promise.resolve([
          {
            appType: "claude",
            generation: 3,
            conflict: true,
            takeoverEnabled: true,
            routeMode: "direct",
          },
          {
            appType: "codex",
            generation: 1,
            conflict: false,
            takeoverEnabled: true,
            routeMode: "proxy",
          },
        ]);
      }
      return Promise.resolve(null);
    });

    const { wrapper } = createWrapper();
    const { result } = renderHook(() => useExternalConfigBridge(), { wrapper });

    await waitFor(() => {
      expect(result.current.currentConflict?.appType).toBe("claude");
    });
    expect(result.current.conflictQueue).toHaveLength(1);
    expect(result.current.currentConflict?.generation).toBe(3);
  });

  it("enqueues on conflict event and upserts generation for same app", async () => {
    const { wrapper } = createWrapper();
    const { result } = renderHook(() => useExternalConfigBridge(), { wrapper });

    await waitFor(() => expect(eventHandler).toBeDefined());

    act(() => {
      eventHandler?.({
        appType: "gemini",
        generation: 2,
        conflict: true,
        takeoverEnabled: true,
      });
    });
    expect(result.current.currentConflict).toEqual({
      appType: "gemini",
      generation: 2,
    });

    // 同 app 新 generation 覆盖旧项，不重复入队
    act(() => {
      eventHandler?.({
        appType: "gemini",
        generation: 5,
        conflict: true,
        takeoverEnabled: true,
      });
    });
    expect(result.current.conflictQueue).toHaveLength(1);
    expect(result.current.currentConflict?.generation).toBe(5);
  });

  it("removes conflict when a resolving event arrives", async () => {
    const { wrapper } = createWrapper();
    const { result } = renderHook(() => useExternalConfigBridge(), { wrapper });

    await waitFor(() => expect(eventHandler).toBeDefined());

    act(() => {
      eventHandler?.({
        appType: "hermes",
        generation: 4,
        conflict: true,
        takeoverEnabled: true,
      });
    });
    expect(result.current.conflictQueue).toHaveLength(1);

    act(() => {
      eventHandler?.({
        appType: "hermes",
        generation: 4,
        conflict: false,
        takeoverEnabled: true,
      });
    });
    expect(result.current.conflictQueue).toHaveLength(0);
  });

  it("dequeues a conflict explicitly after user resolves it", async () => {
    const { wrapper } = createWrapper();
    const { result } = renderHook(() => useExternalConfigBridge(), { wrapper });

    await waitFor(() => expect(eventHandler).toBeDefined());

    act(() => {
      eventHandler?.({
        appType: "opencode",
        generation: 7,
        conflict: true,
        takeoverEnabled: true,
      });
    });
    expect(result.current.conflictQueue).toHaveLength(1);

    act(() => {
      result.current.dequeueConflict("opencode");
    });
    expect(result.current.conflictQueue).toHaveLength(0);
  });
});
