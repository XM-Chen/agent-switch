import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, payload?: Record<string, unknown>) =>
    invokeMock(command, payload),
}));

import { proxyApi } from "./proxy";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe("proxyApi 网关边界", () => {
  it.each([
    ["startProxyServer", "start_proxy_server"],
    ["stopProxyServer", "stop_proxy_server"],
    ["getProxyStatus", "get_proxy_status"],
    ["isProxyRunning", "is_proxy_running"],
    ["getProxyConfig", "get_proxy_config"],
    ["getGatewayAuthStatus", "get_gateway_auth_status"],
  ] as const)("%s 调用 %s", async (method, command) => {
    await proxyApi[method]();
    expect(invokeMock).toHaveBeenCalledWith(command, undefined);
  });

  it("只向网关配置命令传递配置对象", async () => {
    const config = {
      listen_address: "127.0.0.1",
      listen_port: 42567,
      max_retries: 3,
      request_timeout: 600,
      enable_logging: true,
      streaming_first_byte_timeout: 60,
      streaming_idle_timeout: 120,
      non_streaming_timeout: 600,
    };

    await proxyApi.updateProxyConfig(config);
    expect(invokeMock).toHaveBeenCalledWith("update_proxy_config", { config });
  });

  it("映射 API Key 创建与撤销参数", async () => {
    await proxyApi.createGatewayApiKey("本机客户端");
    await proxyApi.revokeGatewayApiKey("key-1");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "create_gateway_api_key", {
      name: "本机客户端",
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "revoke_gateway_api_key", {
      keyId: "key-1",
    });
  });

  it("拒绝关闭独立网关鉴权且不触发 IPC", async () => {
    await expect(proxyApi.setGatewayAuthRequired(false)).rejects.toThrow(
      "独立网关必须启用 Bearer token 鉴权",
    );
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
