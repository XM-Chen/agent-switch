export interface ProxyConfig {
  listen_address: string;
  listen_port: number;
  max_retries: number;
  request_timeout: number;
  enable_logging: boolean;
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
  last_request_at: string | null;
  last_error: string | null;
  failover_count: number;
}

export interface ProxyServerInfo {
  address: string;
  port: number;
  started_at: string;
}

export interface GatewayApiKeySummary {
  id: string;
  name: string;
  keyPrefix: string;
  createdAt: number;
  revokedAt: number | null;
  lastUsedAt: number | null;
}

export interface GatewayAuthStatus {
  authRequired: boolean;
  keys: GatewayApiKeySummary[];
}

export interface CreatedGatewayApiKey {
  key: GatewayApiKeySummary;
  secret: string;
}
