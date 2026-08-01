import { invoke } from "@tauri-apps/api/core";
import type {
  GatewayDomainConfig,
  GatewayModel,
  GatewayModelAlias,
  GatewayMigrationIssue,
  GatewayRoute,
  GatewayRouteHealth,
  GatewayUpstream,
  GatewayUpstreamModel,
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
};
