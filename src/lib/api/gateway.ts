import { invoke } from "@tauri-apps/api/core";
import type {
  CreateGatewayUpstreamInput,
  GatewayDomainConfig,
  GatewayModel,
  GatewayModelAlias,
  GatewayMigrationIssue,
  GatewayRoute,
  GatewayRouteHealth,
  GatewayUpstream,
  GatewayUpstreamModel,
  UpdateGatewayUpstreamInput,
  UpstreamCredentialHint,
} from "@/types/gateway";
export const gatewayApi = {
  getDomainConfig: () =>
    invoke<GatewayDomainConfig>("get_gateway_domain_config"),
  updateDomainConfig: (config: GatewayDomainConfig) =>
    invoke<void>("update_gateway_domain_config", { config }),
  listUpstreams: () => invoke<GatewayUpstream[]>("list_gateway_upstreams"),
  listUpstreamModels: () =>
    invoke<GatewayUpstreamModel[]>("list_gateway_upstream_models"),
  listModels: () => invoke<GatewayModel[]>("list_gateway_models"),
  setModelEnabled: (modelId: string, enabled: boolean) =>
    invoke<boolean>("set_gateway_model_enabled", { modelId, enabled }),
  setModelState: (modelId: string, enabled: boolean, migrationStatus: string) =>
    invoke<boolean>("set_gateway_model_state", {
      modelId,
      enabled,
      migrationStatus,
    }),
  listModelAliases: () =>
    invoke<GatewayModelAlias[]>("list_gateway_model_aliases"),
  listRoutes: () => invoke<GatewayRoute[]>("list_gateway_routes"),
  setRouteEnabled: (routeTargetId: string, enabled: boolean) =>
    invoke<boolean>("set_gateway_route_enabled", { routeTargetId, enabled }),
  reorderRoutes: (gatewayModelId: string, orderedIds: string[]) =>
    invoke<void>("reorder_gateway_routes", { gatewayModelId, orderedIds }),
  listRouteHealth: () =>
    invoke<GatewayRouteHealth[]>("list_gateway_route_health"),
  listMigrationIssues: () =>
    invoke<GatewayMigrationIssue[]>("list_gateway_migration_issues"),
  // 上游 CRUD 与凭据管理。凭据明文仅传入 DPAPI 加密接口，不进 configJson，也不返回。
  createUpstream: (input: CreateGatewayUpstreamInput) =>
    invoke<GatewayUpstream>("create_gateway_upstream", { input }),
  updateUpstream: (upstreamId: string, input: UpdateGatewayUpstreamInput) =>
    invoke<GatewayUpstream>("update_gateway_upstream", { upstreamId, input }),
  deleteUpstream: (upstreamId: string) =>
    invoke<boolean>("delete_gateway_upstream", { upstreamId }),
  setUpstreamEnabled: (upstreamId: string, enabled: boolean) =>
    invoke<GatewayUpstream>("set_gateway_upstream_enabled", {
      upstreamId,
      enabled,
    }),
  listUpstreamCredentials: (upstreamId: string) =>
    invoke<UpstreamCredentialHint[]>("list_gateway_upstream_credential_hints", {
      upstreamId,
    }),
  replaceUpstreamCredential: (
    upstreamId: string,
    credentialKind: string,
    secret: string,
  ) =>
    invoke<UpstreamCredentialHint>("replace_gateway_upstream_credential", {
      upstreamId,
      credentialKind,
      secret,
    }),
  deleteUpstreamCredential: (upstreamId: string, credentialKind: string) =>
    invoke<boolean>("delete_gateway_upstream_credential", {
      upstreamId,
      credentialKind,
    }),
};
