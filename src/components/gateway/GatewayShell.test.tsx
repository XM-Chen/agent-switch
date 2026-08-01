import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { gatewayApiMock, proxyApiMock } = vi.hoisted(() => ({
  gatewayApiMock: {
    getDomainConfig: vi.fn(),
    listUpstreams: vi.fn(),
    listUpstreamModels: vi.fn(),
    listModels: vi.fn(),
    listModelAliases: vi.fn(),
    listRoutes: vi.fn(),
    listMigrationIssues: vi.fn(),
    listRouteHealth: vi.fn(),
    updateDomainConfig: vi.fn(),
    setModelEnabled: vi.fn(),
    setModelState: vi.fn(),
    setRouteEnabled: vi.fn(),
    reorderRoutes: vi.fn(),
  },
  proxyApiMock: {
    getProxyStatus: vi.fn(),
    getGatewayAuthStatus: vi.fn(),
    startProxyServer: vi.fn(),
    stopProxyServer: vi.fn(),
    createGatewayApiKey: vi.fn(),
    revokeGatewayApiKey: vi.fn(),
  },
}));

vi.mock("@/lib/api/gateway", () => ({ gatewayApi: gatewayApiMock }));
vi.mock("@/lib/api/proxy", () => ({ proxyApi: proxyApiMock }));
vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

import { GatewayShell } from "./GatewayShell";

beforeEach(() => {
  gatewayApiMock.getDomainConfig.mockResolvedValue({
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
  });
  gatewayApiMock.listUpstreams.mockResolvedValue([]);
  gatewayApiMock.listUpstreamModels.mockResolvedValue([]);
  gatewayApiMock.listModels.mockResolvedValue([]);
  gatewayApiMock.listModelAliases.mockResolvedValue([]);
  gatewayApiMock.listRoutes.mockResolvedValue([]);
  gatewayApiMock.listMigrationIssues.mockResolvedValue([]);
  gatewayApiMock.listRouteHealth.mockResolvedValue([]);
  proxyApiMock.getProxyStatus.mockResolvedValue({
    running: false,
    address: "127.0.0.1",
    port: 42567,
    active_connections: 0,
    total_requests: 0,
    success_requests: 0,
    failed_requests: 0,
    success_rate: 0,
    uptime_seconds: 0,
    last_request_at: null,
    last_error: null,
    failover_count: 0,
  });
  proxyApiMock.getGatewayAuthStatus.mockResolvedValue({
    authRequired: true,
    keys: [],
  });
});

describe("GatewayShell", () => {
  it("只暴露网关控制面，不再渲染客户端工具箱入口", async () => {
    render(<GatewayShell />);

    expect(await screen.findByText("本地模型网关")).toBeInTheDocument();
    for (const retiredEntry of [
      "MCP",
      "Prompts",
      "Skills",
      "Sessions",
      "Workspace",
      "接管配置",
    ]) {
      expect(screen.queryByText(retiredEntry)).not.toBeInTheDocument();
    }
  });

  it("接入文档只展示可复制的规范协议入口", async () => {
    const user = userEvent.setup();
    render(<GatewayShell />);

    await screen.findByText("本地模型网关");
    await user.click(screen.getByRole("button", { name: "接入文档" }));

    expect(
      screen.getByDisplayValue("http://127.0.0.1:42567/v1/messages"),
    ).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("http://127.0.0.1:42567/v1/chat/completions"),
    ).toBeInTheDocument();
    expect(
      screen.getByDisplayValue("http://127.0.0.1:42567/v1/responses"),
    ).toBeInTheDocument();
  });
});
