import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, payload?: Record<string, unknown>) =>
    invokeMock(command, payload),
}));

import { gatewayApi } from "./gateway";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("gatewayApi 命令映射", () => {
  it.each([
    ["getDomainConfig", "get_gateway_domain_config"],
    ["listUpstreams", "list_gateway_upstreams"],
    ["listUpstreamModels", "list_gateway_upstream_models"],
    ["listModels", "list_gateway_models"],
    ["listModelAliases", "list_gateway_model_aliases"],
    ["listRoutes", "list_gateway_routes"],
    ["listMigrationIssues", "list_gateway_migration_issues"],
    ["listRouteHealth", "list_gateway_route_health"],
  ] as const)("%s 调用 %s", async (method, command) => {
    await gatewayApi[method]();
    expect(invokeMock).toHaveBeenCalledWith(command, undefined);
  });

  it("按 camelCase 参数更新网关配置", async () => {
    const config = {
      authRequired: true,
      listenAddress: "127.0.0.1",
      listenPort: 42567,
      enableLogging: true,
      maxRetries: 3,
      streamingFirstByteTimeout: 60,
      streamingIdleTimeout: 120,
      nonStreamingTimeout: 600,
      circuitFailureThreshold: 3,
      circuitSuccessThreshold: 2,
      circuitTimeoutSeconds: 60,
      circuitErrorRateThreshold: 0.5,
      circuitMinRequests: 5,
    };

    await gatewayApi.updateDomainConfig(config);
    expect(invokeMock).toHaveBeenCalledWith("update_gateway_domain_config", {
      config,
    });
  });

  it("区分 active 模型普通开关与非 active 模型确认激活", async () => {
    await gatewayApi.setModelEnabled("model-1", false);
    await gatewayApi.setModelState("model-2", true, "active");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "set_gateway_model_enabled", {
      modelId: "model-1",
      enabled: false,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "set_gateway_model_state", {
      modelId: "model-2",
      enabled: true,
      migrationStatus: "active",
    });
  });

  it("映射路由开关与同模型候选重排参数", async () => {
    await gatewayApi.setRouteEnabled("route-1", true);
    await gatewayApi.reorderRoutes("model-1", ["route-2", "route-1"]);

    expect(invokeMock).toHaveBeenNthCalledWith(1, "set_gateway_route_enabled", {
      routeTargetId: "route-1",
      enabled: true,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "reorder_gateway_routes", {
      gatewayModelId: "model-1",
      orderedIds: ["route-2", "route-1"],
    });
  });
});
