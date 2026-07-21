import { invoke } from "@tauri-apps/api/core";
import type {
  ProxyConfig,
  ProxyStatus,
  ProxyServerInfo,
  ProxyTakeoverStatus,
  GlobalProxyConfig,
  AppProxyConfig,
  ProxyRouteMode,
  ExternalConfigModuleStatus,
} from "@/types/proxy";

export const proxyApi = {
  // ========== 代理服务器控制 API ==========

  // 启动代理服务器
  async startProxyServer(): Promise<ProxyServerInfo> {
    return invoke("start_proxy_server");
  },

  // 受保护地停止代理服务器，不改写任何模块 Live 配置
  async stopProxyServer(): Promise<void> {
    return invoke("stop_proxy_server");
  },

  // 旧调用名兼容，后端语义同样是受保护的纯网关停止
  async stopProxyWithRestore(): Promise<void> {
    return invoke("stop_proxy_server");
  },

  // 获取代理服务器状态
  async getProxyStatus(): Promise<ProxyStatus> {
    return invoke("get_proxy_status");
  },

  // 检查代理服务器是否正在运行
  async isProxyRunning(): Promise<boolean> {
    return invoke("is_proxy_running");
  },

  // 检查是否处于接管模式
  async isLiveTakeoverActive(): Promise<boolean> {
    return invoke("is_live_takeover_active");
  },

  // 代理模式下切换供应商
  async switchProxyProvider(
    appType: string,
    providerId: string,
  ): Promise<void> {
    return invoke("switch_proxy_provider", { appType, providerId });
  },

  // ========== 接管状态 API ==========

  // 获取各应用接管状态
  async getProxyTakeoverStatus(): Promise<ProxyTakeoverStatus> {
    return invoke("get_proxy_takeover_status");
  },

  // 为指定应用开启/关闭接管
  // 开启时可携带 routeMode（缺省由后端按 direct 处理）
  async setProxyTakeoverForApp(
    appType: string,
    enabled: boolean,
    routeMode?: ProxyRouteMode,
  ): Promise<void> {
    return invoke("set_proxy_takeover_for_app", {
      appType,
      enabled,
      routeMode,
    });
  },

  // 切换指定应用的路由模式（偏好/热切换；未接管时后端只存偏好）
  async setProxyRouteMode(
    appType: string,
    routeMode: ProxyRouteMode,
  ): Promise<void> {
    return invoke("set_proxy_route_mode", { appType, routeMode });
  },

  // ========== 外部配置检测与冲突 API ==========

  // 获取七模块外部配置状态（含冲突态），启动时用于 hydrate 冲突队列
  async getExternalConfigStatus(): Promise<ExternalConfigModuleStatus[]> {
    return invoke("get_external_config_status");
  },

  // 接受外部更改：保留外部配置为新的受管基线，按实际路由同步 routeMode
  async acceptExternalConfigChange(
    appType: string,
    generation: number,
  ): Promise<void> {
    return invoke("accept_external_config_change", { appType, generation });
  },

  // 拒绝外部更改：用 Agent-Switch 受管配置重新覆盖 live
  async rejectExternalConfigChange(
    appType: string,
    generation: number,
  ): Promise<void> {
    return invoke("reject_external_config_change", { appType, generation });
  },

  // ========== Legacy 代理配置 API (兼容) ==========

  // 获取代理配置（旧版 v2 兼容接口）
  async getProxyConfig(): Promise<ProxyConfig> {
    return invoke("get_proxy_config");
  },

  // 更新代理配置（旧版 v2 兼容接口）
  async updateProxyConfig(config: ProxyConfig): Promise<void> {
    return invoke("update_proxy_config", { config });
  },

  // ========== v3+ 全局/应用级配置 API ==========

  // 获取全局代理配置
  async getGlobalProxyConfig(): Promise<GlobalProxyConfig> {
    return invoke("get_global_proxy_config");
  },

  // 更新全局代理配置
  async updateGlobalProxyConfig(config: GlobalProxyConfig): Promise<void> {
    return invoke("update_global_proxy_config", { config });
  },

  // 获取指定应用的代理配置
  async getProxyConfigForApp(appType: string): Promise<AppProxyConfig> {
    return invoke("get_proxy_config_for_app", { appType });
  },

  // 更新指定应用的代理配置
  async updateProxyConfigForApp(config: AppProxyConfig): Promise<void> {
    return invoke("update_proxy_config_for_app", { config });
  },

  // ========== 计费默认配置 API ==========

  // 获取默认成本倍率
  async getDefaultCostMultiplier(appType: string): Promise<string> {
    return invoke("get_default_cost_multiplier", { appType });
  },

  // 设置默认成本倍率
  async setDefaultCostMultiplier(
    appType: string,
    value: string,
  ): Promise<void> {
    return invoke("set_default_cost_multiplier", { appType, value });
  },

  // 获取计费模式来源
  async getPricingModelSource(appType: string): Promise<string> {
    return invoke("get_pricing_model_source", { appType });
  },

  // 设置计费模式来源
  async setPricingModelSource(appType: string, value: string): Promise<void> {
    return invoke("set_pricing_model_source", { appType, value });
  },
};
