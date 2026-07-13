import { describe, expect, it, vi, beforeEach } from "vitest";

// 本文件用本地 vi.mock 覆盖全局 tauri invoke mock，直接捕获命令名与参数，
// 验证 setAggregationEnabled / setTierSelection 的「读-改-写整个 CcAggregateConfig」逻辑。
const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, payload?: Record<string, unknown>) =>
    invokeMock(command, payload),
}));

import { aggregationApi, type CcAggregateConfig } from "./aggregation";

beforeEach(() => {
  invokeMock.mockReset();
});

describe("aggregationApi command mapping", () => {
  it("passes null providerId when omitted", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await aggregationApi.listProviderModels("claude");
    expect(invokeMock).toHaveBeenCalledWith("list_provider_models", {
      appType: "claude",
      providerId: null,
    });
  });

  it("maps updateCustomAggregate with null for omitted fields and no appType", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await aggregationApi.updateCustomAggregate("id-1", "new name");
    expect(invokeMock).toHaveBeenCalledWith("update_custom_aggregate", {
      id: "id-1",
      name: "new name",
      members: null,
    });
  });
});

describe("setAggregationEnabled (read-modify-write)", () => {
  it("preserves tierSelection while flipping enabled", async () => {
    const current: CcAggregateConfig = {
      enabled: false,
      tierSelection: { sonnet: { type: "auto", value: "glm-4.6" } },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_cc_aggregate_config")
        return Promise.resolve(current);
      return Promise.resolve(undefined);
    });

    await aggregationApi.setAggregationEnabled("claude", true);

    expect(invokeMock).toHaveBeenCalledWith("set_cc_aggregate_config", {
      appType: "claude",
      config: {
        enabled: true,
        tierSelection: { sonnet: { type: "auto", value: "glm-4.6" } },
      },
    });
  });
});

describe("setTierSelection (read-modify-write)", () => {
  it("sets a single tier without clobbering others", async () => {
    const current: CcAggregateConfig = {
      enabled: true,
      tierSelection: { sonnet: { type: "auto", value: "a" } },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_cc_aggregate_config")
        return Promise.resolve(current);
      return Promise.resolve(undefined);
    });

    await aggregationApi.setTierSelection("claude", "opus", {
      type: "custom",
      value: "cust-1",
    });

    expect(invokeMock).toHaveBeenCalledWith("set_cc_aggregate_config", {
      appType: "claude",
      config: {
        enabled: true,
        tierSelection: {
          sonnet: { type: "auto", value: "a" },
          opus: { type: "custom", value: "cust-1" },
        },
      },
    });
  });

  it("clears a tier when ref is null", async () => {
    const current: CcAggregateConfig = {
      enabled: true,
      tierSelection: {
        sonnet: { type: "auto", value: "a" },
        opus: { type: "custom", value: "cust-1" },
      },
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_cc_aggregate_config")
        return Promise.resolve(current);
      return Promise.resolve(undefined);
    });

    await aggregationApi.setTierSelection("claude", "opus", null);

    expect(invokeMock).toHaveBeenCalledWith("set_cc_aggregate_config", {
      appType: "claude",
      config: {
        enabled: true,
        tierSelection: { sonnet: { type: "auto", value: "a" } },
      },
    });
  });
});
