export interface GatewayDomainConfig {
  authRequired: boolean;
  listenAddress: string;
  listenPort: number;
  enableLogging: boolean;
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
export interface GatewayUpstream {
  id: string;
  name: string;
  enabled: boolean;
  baseUrl: string | null;
  protocol: string;
  adapterType: string;
  configJson: unknown;
  notes: string | null;
  legacyAppType: string | null;
  legacyProviderId: string | null;
  createdAt: number;
  updatedAt: number;
}
export interface GatewayUpstreamModel {
  upstreamId: string;
  modelId: string;
  source: string;
  ownedBy: string | null;
  refreshedAt: number;
  legacyAppType: string | null;
  legacyProviderId: string | null;
}
export interface GatewayModel {
  id: string;
  modelId: string;
  displayName: string;
  enabled: boolean;
  source: string;
  migrationStatus: string;
  legacyAppType: string | null;
  legacySourceId: string | null;
  metadataJson: unknown;
  createdAt: number;
  updatedAt: number;
}
export interface GatewayModelAlias {
  alias: string;
  gatewayModelId: string;
  createdAt: number;
}
export interface GatewayRoute {
  id: string;
  gatewayModelId: string;
  upstreamId: string;
  targetModel: string;
  position: number;
  enabled: boolean;
  legacyAppType: string | null;
  legacyAggregateId: string | null;
  createdAt: number;
  updatedAt: number;
}
export interface GatewayRouteHealth {
  routeTargetId: string;
  state: string;
  consecutiveFailures: number;
  consecutiveSuccesses: number;
  lastSuccessAt: number | null;
  lastFailureAt: number | null;
  openedAt: number | null;
  lastError: string | null;
  updatedAt: number;
}
export interface GatewayMigrationIssue {
  migrationKey: string;
  severity: string;
  entityType: string;
  legacyAppType: string | null;
  legacyEntityId: string | null;
  code: string;
  detailsJson: unknown;
  createdAt: number;
}

/** 创建上游的输入。`configJson` 仅存放非敏感 adapter 配置，凭据走独立接口。 */
export interface CreateGatewayUpstreamInput {
  name: string;
  enabled: boolean;
  baseUrl: string;
  protocol: string;
  adapterType: string;
  configJson: unknown;
  notes?: string | null;
}

/** 更新上游的输入。enabled 通过单独的启停命令切换，不在此处提交。 */
export interface UpdateGatewayUpstreamInput {
  name: string;
  baseUrl: string;
  protocol: string;
  adapterType: string;
  configJson: unknown;
  notes?: string | null;
}

/** 上游凭据的只读展示。明文从不返回，仅 `keyHint` 给出脱敏提示。 */
export interface UpstreamCredentialHint {
  id: string;
  upstreamId: string;
  credentialKind: string;
  encryptionScheme: string;
  keyHint: string | null;
  createdAt: number;
  updatedAt: number;
}
