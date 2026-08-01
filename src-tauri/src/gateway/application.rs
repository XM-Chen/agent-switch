//! 独立网关路由应用层。
//!
//! SQLite 适配器只把影子领域记录投影成 core 端口；HTTP/forwarder 在下一步接入。

use std::str::FromStr;

use crate::database::Database;

use super::core::{
    GatewayModelRoute, IngressProtocol, ModelRouter, RouteCatalog, RouteResolutionError,
    RouteTarget, TargetHealth, UpstreamProtocol,
};

pub struct DatabaseRouteCatalog<'a> {
    db: &'a Database,
}

impl<'a> DatabaseRouteCatalog<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    fn project(record: crate::database::GatewayModelRecord) -> GatewayModelRoute {
        GatewayModelRoute {
            gateway_model_id: record.id,
            model_id: record.model_id,
        }
    }
}

impl RouteCatalog for DatabaseRouteCatalog<'_> {
    fn resolve_model_exact(&self, requested_model: &str) -> Option<GatewayModelRoute> {
        self.db
            .get_enabled_gateway_model_by_model_id(requested_model)
            .ok()?
            .map(Self::project)
    }

    fn resolve_alias_exact(&self, alias: &str) -> Option<GatewayModelRoute> {
        self.db
            .get_enabled_gateway_model_by_alias(alias)
            .ok()?
            .map(Self::project)
    }

    fn list_targets(&self, gateway_model_id: &str) -> Vec<RouteTarget> {
        self.db
            .list_enabled_route_targets_with_protocol(gateway_model_id)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(target, protocol)| {
                Some(RouteTarget {
                    route_target_id: target.id,
                    gateway_model_id: target.gateway_model_id,
                    upstream_id: target.upstream_id,
                    target_model: target.target_model,
                    position: target.position,
                    upstream_protocol: UpstreamProtocol::from_str(&protocol).ok()?,
                })
            })
            .collect()
    }
}

pub struct DatabaseTargetHealth<'a> {
    db: &'a Database,
}

impl<'a> DatabaseTargetHealth<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }
}

impl TargetHealth for DatabaseTargetHealth<'_> {
    fn is_available(&self, route_target_id: &str) -> bool {
        self.db
            .route_target_is_closed_or_missing(route_target_id)
            .unwrap_or(false)
    }
}

pub fn resolve_database_route(
    db: &Database,
    ingress: IngressProtocol,
    requested_model: &str,
) -> Result<(GatewayModelRoute, Vec<RouteTarget>), RouteResolutionError> {
    let catalog = DatabaseRouteCatalog::new(db);
    let health = DatabaseTargetHealth::new(db);
    ModelRouter::resolve(&catalog, &health, ingress, requested_model)
}

#[cfg(test)]
mod tests {
    use crate::database::Database;
    use rusqlite::params;

    use super::*;

    #[test]
    fn database_adapter_resolves_exact_model_and_alias_without_current_provider() {
        let db = Database::memory().expect("memory db");
        let now = chrono::Utc::now().timestamp_millis();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute(
                "INSERT INTO upstreams
                    (id, name, enabled, protocol, adapter_type, created_at, updated_at)
                 VALUES ('up-1', 'Upstream', 1, 'anthropic', 'claude', ?1, ?1)",
                [now],
            )
            .expect("insert upstream");
            conn.execute(
                "INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status,
                     created_at, updated_at)
                 VALUES ('gm-1', 'stable-model', 'Stable', 1, 'manual', 'active', ?1, ?1)",
                [now],
            )
            .expect("insert model");
            conn.execute(
                "INSERT INTO model_aliases (alias, gateway_model_id, created_at)
                 VALUES ('model-alias', 'gm-1', ?1)",
                [now],
            )
            .expect("insert alias");
            conn.execute(
                "INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     created_at, updated_at)
                 VALUES ('target-1', 'gm-1', 'up-1', 'vendor-model', 0, 1, ?1, ?1)",
                [now],
            )
            .expect("insert target");
        }

        for request_model in ["stable-model", "model-alias"] {
            let (_, targets) =
                resolve_database_route(&db, IngressProtocol::AnthropicMessages, request_model)
                    .expect("resolve route");
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].route_target_id, "target-1");
        }
    }

    #[test]
    fn database_adapter_filters_open_route_target() {
        let db = Database::memory().expect("memory db");
        let now = chrono::Utc::now().timestamp_millis();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute_batch(&format!(
                "INSERT INTO upstreams
                    (id, name, enabled, protocol, adapter_type, created_at, updated_at)
                 VALUES ('up-1', 'Upstream', 1, 'openai_responses', 'codex', {now}, {now});
                 INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status,
                     created_at, updated_at)
                 VALUES ('gm-1', 'stable-model', 'Stable', 1, 'manual', 'active', {now}, {now});
                 INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     created_at, updated_at)
                 VALUES ('target-1', 'gm-1', 'up-1', 'vendor-model', 0, 1, {now}, {now});
                 INSERT INTO route_target_health
                    (route_target_id, state, updated_at)
                 VALUES ('target-1', 'open', {now});"
            ))
            .expect("seed open target");
        }

        let error = resolve_database_route(&db, IngressProtocol::OpenAiResponses, "stable-model")
            .expect_err("open target unavailable");
        assert!(matches!(
            error,
            RouteResolutionError::NoAvailableTarget { .. }
        ));
    }
}
