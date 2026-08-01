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
