export interface ProxyConfig {
  listen_address: string;
  listen_port: number;
  max_retries: number;
  request_timeout: number;
  enable_logging: boolean;
  live_takeover_active?: boolean;
  // 超时配置
  streaming_first_byte_timeout: number;
  streaming_idle_timeout: number;
  non_streaming_timeout: number;
}

export interface ProxyStatus {
  running: boolean;
  address: string;
  port: number;
  active_connections: number;
  total_requests: number;
  success_requests: number;
  failed_requests: number;
  success_rate: number;
  uptime_seconds: number;
  current_provider: string | null;
  current_provider_id: string | null;
  last_request_at: string | null;
  last_error: string | null;
  failover_count: number;
  active_targets?: ActiveTarget[];
}

export interface ActiveTarget {
  app_type: string;
  provider_name: string;
  provider_id: string;
}

export interface ProxyServerInfo {
  address: string;
  port: number;
  started_at: string;
}

export type ProxyRouteMode = "direct" | "proxy";

export interface ProxyModuleTakeoverStatus {
  takeoverEnabled: boolean;
  routeMode: ProxyRouteMode;
}

export interface ProxyTakeoverStatus {
  claude: ProxyModuleTakeoverStatus;
  claudeDesktop: ProxyModuleTakeoverStatus;
  codex: ProxyModuleTakeoverStatus;
  gemini: ProxyModuleTakeoverStatus;
  opencode: ProxyModuleTakeoverStatus;
  openclaw: ProxyModuleTakeoverStatus;
  hermes: ProxyModuleTakeoverStatus;
}

export const PROXY_TAKEOVER_STATUS_KEY = {
  claude: "claude",
  "claude-desktop": "claudeDesktop",
  codex: "codex",
  gemini: "gemini",
  opencode: "opencode",
  openclaw: "openclaw",
  hermes: "hermes",
} as const satisfies Record<string, keyof ProxyTakeoverStatus>;

export function getProxyTakeoverState(
  status: ProxyTakeoverStatus | undefined,
  appType: string,
): ProxyModuleTakeoverStatus | undefined {
  const key =
    PROXY_TAKEOVER_STATUS_KEY[
      appType as keyof typeof PROXY_TAKEOVER_STATUS_KEY
    ];
  return key && status ? status[key] : undefined;
}

export interface ProxyStopError {
  code: "proxyRoutesActive" | "stopFailed" | string;
  message: string;
  modules: string[];
}

// C3/C4 外部配置检测与冲突
// 事件载荷（`external-config-changed`），后端已输出 camelCase
export interface ExternalConfigChangedPayload {
  appType: string; // 规范值，如 "claude-desktop"
  generation: number;
  conflict: boolean;
  takeoverEnabled: boolean;
}

// `get_external_config_status` 返回的模块状态项
export interface ExternalConfigModuleStatus {
  appType: string; // 规范值，如 "claude-desktop"
  generation: number;
  conflict: boolean;
  takeoverEnabled: boolean;
  routeMode: ProxyRouteMode;
}

export interface ProviderHealth {
  provider_id: string;
  app_type: string;
  is_healthy: boolean;
  consecutive_failures: number;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error: string | null;
  updated_at: string;
}

// 熔断器相关类型
export interface CircuitBreakerConfig {
  failureThreshold: number;
  successThreshold: number;
  timeoutSeconds: number;
  errorRateThreshold: number;
  minRequests: number;
}

export type CircuitState = "closed" | "open" | "half_open";

export interface CircuitBreakerStats {
  state: CircuitState;
  consecutiveFailures: number;
  consecutiveSuccesses: number;
  totalRequests: number;
  failedRequests: number;
}

// 供应商健康状态枚举
export enum ProviderHealthStatus {
  Healthy = "healthy",
  Degraded = "degraded",
  Failed = "failed",
  Unknown = "unknown",
}

// 扩展 ProviderHealth 以包含前端计算的状态
export interface ProviderHealthWithStatus extends ProviderHealth {
  status: ProviderHealthStatus;
  circuitState?: CircuitState;
}

export interface ProxyUsageRecord {
  provider_id: string;
  app_type: string;
  endpoint: string;
  request_tokens: number | null;
  response_tokens: number | null;
  status_code: number;
  latency_ms: number;
  error: string | null;
  timestamp: string;
}

// 故障转移队列条目
export interface FailoverQueueItem {
  providerId: string;
  providerName: string;
  providerNotes?: string;
  sortIndex?: number;
}

// 全局代理配置（统一字段，三行镜像）
export interface GlobalProxyConfig {
  proxyEnabled: boolean;
  listenAddress: string;
  listenPort: number;
  enableLogging: boolean;
}

// 应用级代理配置（每个 app 独立）
export interface AppProxyConfig {
  appType: string;
  takeoverEnabled: boolean;
  routeMode: ProxyRouteMode;
  autoFailoverEnabled: boolean;
  maxRetries: number;
  streamingFirstByteTimeout: number;
  streamingIdleTimeout: number;
  nonStreamingTimeout: number;
  circuitFailureThreshold: number;
  circuitSuccessThreshold: number;
  circuitTimeoutSeconds: number;
  circuitErrorRateThreshold: number;
  circuitMinRequests: number;
}
