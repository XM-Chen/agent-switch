import { invoke } from "@tauri-apps/api/core";
import type {
  CreatedGatewayApiKey,
  GatewayAuthStatus,
  ProxyConfig,
  ProxyServerInfo,
  ProxyStatus,
} from "@/types/proxy";

export const proxyApi = {
  startProxyServer: () => invoke<ProxyServerInfo>("start_proxy_server"),
  stopProxyServer: () => invoke<void>("stop_proxy_server"),
  getProxyStatus: () => invoke<ProxyStatus>("get_proxy_status"),
  isProxyRunning: () => invoke<boolean>("is_proxy_running"),
  getProxyConfig: () => invoke<ProxyConfig>("get_proxy_config"),
  updateProxyConfig: (config: ProxyConfig) =>
    invoke<void>("update_proxy_config", { config }),
  getGatewayAuthStatus: () =>
    invoke<GatewayAuthStatus>("get_gateway_auth_status"),
  createGatewayApiKey: (name: string) =>
    invoke<CreatedGatewayApiKey>("create_gateway_api_key", { name }),
  revokeGatewayApiKey: (keyId: string) =>
    invoke<boolean>("revoke_gateway_api_key", { keyId }),
  setGatewayAuthRequired: (required: boolean) => {
    if (!required) {
      return Promise.reject(new Error("独立网关必须启用 Bearer token 鉴权"));
    }
    return invoke<void>("set_gateway_auth_required", { required });
  },
};
