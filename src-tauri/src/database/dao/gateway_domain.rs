//! 独立网关影子领域 DAO。
//!
//! 阶段 2 只建立新领域的持久化边界；生产路由在阶段 3 才切换到这些表。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfigRecord {
    pub auth_required: bool,
    pub listen_address: String,
    pub listen_port: u16,
    pub enable_logging: bool,
    pub max_retries: u8,
    pub streaming_first_byte_timeout: u64,
    pub streaming_idle_timeout: u64,
    pub non_streaming_timeout: u64,
    pub circuit_failure_threshold: u32,
    pub circuit_success_threshold: u32,
    pub circuit_timeout_seconds: u64,
    pub circuit_error_rate_threshold: f64,
    pub circuit_min_requests: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamRecord {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    pub protocol: String,
    pub adapter_type: String,
    pub config_json: serde_json::Value,
    pub notes: Option<String>,
    pub legacy_app_type: Option<String>,
    pub legacy_provider_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamCredentialRecord {
    pub id: String,
    pub upstream_id: String,
    pub credential_kind: String,
    pub encrypted_payload: Vec<u8>,
    pub encryption_scheme: String,
    pub key_hint: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamModelRecord {
    pub upstream_id: String,
    pub model_id: String,
    pub source: String,
    pub owned_by: Option<String>,
    pub refreshed_at: i64,
    pub legacy_app_type: Option<String>,
    pub legacy_provider_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayModelRecord {
    pub id: String,
    pub model_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub source: String,
    pub migration_status: String,
    pub legacy_app_type: Option<String>,
    pub legacy_source_id: Option<String>,
    pub metadata_json: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelAliasRecord {
    pub alias: String,
    pub gateway_model_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteTargetRecord {
    pub id: String,
    pub gateway_model_id: String,
    pub upstream_id: String,
    pub target_model: String,
    pub position: i64,
    pub enabled: bool,
    pub legacy_app_type: Option<String>,
    pub legacy_aggregate_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteTargetHealthRecord {
    pub route_target_id: String,
    pub state: String,
    pub consecutive_failures: i64,
    pub consecutive_successes: i64,
    pub last_success_at: Option<i64>,
    pub last_failure_at: Option<i64>,
    pub opened_at: Option<i64>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayMigrationIssue {
    pub migration_key: String,
    pub severity: String,
    pub entity_type: String,
    pub legacy_app_type: Option<String>,
    pub legacy_entity_id: Option<String>,
    pub code: String,
    pub details_json: serde_json::Value,
    pub created_at: i64,
}

fn parse_json_column(raw: String, field: &str) -> rusqlite::Result<serde_json::Value> {
    serde_json::from_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{field} 不是合法 JSON: {error}"),
            )),
        )
    })
}

impl Database {
    pub fn get_gateway_config_record(&self) -> Result<GatewayConfigRecord, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT auth_required, listen_address, listen_port, enable_logging, max_retries,
                    streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout,
                    circuit_failure_threshold, circuit_success_threshold, circuit_timeout_seconds,
                    circuit_error_rate_threshold, circuit_min_requests
             FROM gateway_config WHERE id = 1",
            [],
            |row| {
                Ok(GatewayConfigRecord {
                    auth_required: row.get::<_, i64>(0)? != 0,
                    listen_address: row.get(1)?,
                    listen_port: row.get::<_, u16>(2)?,
                    enable_logging: row.get::<_, i64>(3)? != 0,
                    max_retries: row.get::<_, u8>(4)?,
                    streaming_first_byte_timeout: row.get::<_, u64>(5)?,
                    streaming_idle_timeout: row.get::<_, u64>(6)?,
                    non_streaming_timeout: row.get::<_, u64>(7)?,
                    circuit_failure_threshold: row.get::<_, u32>(8)?,
                    circuit_success_threshold: row.get::<_, u32>(9)?,
                    circuit_timeout_seconds: row.get::<_, u64>(10)?,
                    circuit_error_rate_threshold: row.get(11)?,
                    circuit_min_requests: row.get::<_, u32>(12)?,
                })
            },
        )
        .map_err(|e| AppError::Database(format!("读取独立网关配置失败: {e}")))
    }

    /// 更新项目自有网关运行配置；不会读取或写入任何客户端配置。
    pub fn update_gateway_config_record(
        &self,
        config: &GatewayConfigRecord,
    ) -> Result<(), AppError> {
        if !config.auth_required {
            return Err(AppError::InvalidInput(
                "独立网关必须启用 Bearer token 鉴权".to_string(),
            ));
        }
        if config.listen_address != "127.0.0.1" {
            return Err(AppError::InvalidInput(
                "独立网关仅允许监听 127.0.0.1".to_string(),
            ));
        }
        if config.listen_port == 0 {
            return Err(AppError::InvalidInput("监听端口不能为 0".to_string()));
        }
        if config.streaming_first_byte_timeout == 0
            || config.streaming_idle_timeout == 0
            || config.non_streaming_timeout == 0
            || config.circuit_timeout_seconds == 0
        {
            return Err(AppError::InvalidInput("超时配置必须大于 0".to_string()));
        }
        if config.circuit_failure_threshold == 0
            || config.circuit_success_threshold == 0
            || config.circuit_min_requests == 0
        {
            return Err(AppError::InvalidInput("熔断阈值必须大于 0".to_string()));
        }
        if !config.circuit_error_rate_threshold.is_finite()
            || !(0.0..=1.0).contains(&config.circuit_error_rate_threshold)
        {
            return Err(AppError::InvalidInput(
                "熔断错误率阈值必须位于 0 到 1 之间".to_string(),
            ));
        }

        let conn = lock_conn!(self.conn);
        let updated = conn.execute(
            "UPDATE gateway_config SET auth_required = ?1, listen_address = ?2, listen_port = ?3,
                    enable_logging = ?4, max_retries = ?5, streaming_first_byte_timeout = ?6,
                    streaming_idle_timeout = ?7, non_streaming_timeout = ?8,
                    circuit_failure_threshold = ?9, circuit_success_threshold = ?10,
                    circuit_timeout_seconds = ?11, circuit_error_rate_threshold = ?12,
                    circuit_min_requests = ?13, updated_at = ?14 WHERE id = 1",
            params![
                if config.auth_required { 1 } else { 0 }, config.listen_address, config.listen_port,
                if config.enable_logging { 1 } else { 0 }, config.max_retries,
                config.streaming_first_byte_timeout, config.streaming_idle_timeout,
                config.non_streaming_timeout, config.circuit_failure_threshold,
                config.circuit_success_threshold, config.circuit_timeout_seconds,
                config.circuit_error_rate_threshold, config.circuit_min_requests,
                chrono::Utc::now().timestamp_millis(),
            ],
        ).map_err(|e| AppError::Database(format!("更新独立网关配置失败: {e}")))?;
        if updated != 1 {
            return Err(AppError::Database("独立网关配置不存在".to_string()));
        }
        Ok(())
    }

    pub fn set_gateway_model_enabled(
        &self,
        model_id: &str,
        enabled: bool,
    ) -> Result<bool, AppError> {
        self.set_gateway_model_enabled_strict(model_id, enabled)
    }

    pub fn set_gateway_model_state(
        &self,
        model_id: &str,
        enabled: bool,
        migration_status: &str,
    ) -> Result<bool, AppError> {
        self.set_gateway_model_state_strict(model_id, enabled, migration_status)
    }

    pub fn set_route_target_enabled(
        &self,
        target_id: &str,
        enabled: bool,
    ) -> Result<bool, AppError> {
        self.set_route_target_enabled_strict(target_id, enabled)
    }

    pub fn reorder_route_targets(
        &self,
        gateway_model_id: &str,
        ordered_ids: &[String],
    ) -> Result<(), AppError> {
        self.reorder_route_targets_strict(gateway_model_id, ordered_ids)
    }

    pub fn list_upstreams(&self) -> Result<Vec<UpstreamRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, enabled, base_url, protocol, adapter_type, config_json, notes,
                        legacy_app_type, legacy_provider_id, created_at, updated_at
                 FROM upstreams ORDER BY name ASC, id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取上游失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(UpstreamRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    base_url: row.get(3)?,
                    protocol: row.get(4)?,
                    adapter_type: row.get(5)?,
                    config_json: parse_json_column(row.get(6)?, "upstreams.config_json")?,
                    notes: row.get(7)?,
                    legacy_app_type: row.get(8)?,
                    legacy_provider_id: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取上游失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析上游失败: {e}")))
    }

    pub fn get_upstream(&self, id: &str) -> Result<Option<UpstreamRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, name, enabled, base_url, protocol, adapter_type, config_json, notes,
                    legacy_app_type, legacy_provider_id, created_at, updated_at
             FROM upstreams WHERE id = ?1",
            [id],
            |row| {
                Ok(UpstreamRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    base_url: row.get(3)?,
                    protocol: row.get(4)?,
                    adapter_type: row.get(5)?,
                    config_json: parse_json_column(row.get(6)?, "upstreams.config_json")?,
                    notes: row.get(7)?,
                    legacy_app_type: row.get(8)?,
                    legacy_provider_id: row.get(9)?,
                    created_at: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(format!("读取上游失败: {e}")))
    }

    pub fn list_upstream_models(&self) -> Result<Vec<UpstreamModelRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT upstream_id, model_id, source, owned_by, refreshed_at,
                        legacy_app_type, legacy_provider_id
                 FROM upstream_models ORDER BY model_id ASC, upstream_id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取上游模型失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(UpstreamModelRecord {
                    upstream_id: row.get(0)?,
                    model_id: row.get(1)?,
                    source: row.get(2)?,
                    owned_by: row.get(3)?,
                    refreshed_at: row.get(4)?,
                    legacy_app_type: row.get(5)?,
                    legacy_provider_id: row.get(6)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取上游模型失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析上游模型失败: {e}")))
    }

    pub fn list_gateway_models(&self) -> Result<Vec<GatewayModelRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, model_id, display_name, enabled, source, migration_status,
                        legacy_app_type, legacy_source_id, metadata_json, created_at, updated_at
                 FROM gateway_models ORDER BY model_id ASC, id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取网关模型失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GatewayModelRecord {
                    id: row.get(0)?,
                    model_id: row.get(1)?,
                    display_name: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    source: row.get(4)?,
                    migration_status: row.get(5)?,
                    legacy_app_type: row.get(6)?,
                    legacy_source_id: row.get(7)?,
                    metadata_json: parse_json_column(row.get(8)?, "gateway_models.metadata_json")?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取网关模型失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析网关模型失败: {e}")))
    }

    pub(crate) fn get_enabled_gateway_model_by_model_id(
        &self,
        requested_model: &str,
    ) -> Result<Option<GatewayModelRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, model_id, display_name, enabled, source, migration_status,
                    legacy_app_type, legacy_source_id, metadata_json, created_at, updated_at
             FROM gateway_models
             WHERE model_id = ?1 AND enabled = 1 AND migration_status = 'active'
             ORDER BY id ASC LIMIT 1",
            [requested_model],
            |row| {
                Ok(GatewayModelRecord {
                    id: row.get(0)?,
                    model_id: row.get(1)?,
                    display_name: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    source: row.get(4)?,
                    migration_status: row.get(5)?,
                    legacy_app_type: row.get(6)?,
                    legacy_source_id: row.get(7)?,
                    metadata_json: parse_json_column(row.get(8)?, "gateway_models.metadata_json")?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(format!("解析精确网关模型失败: {e}")))
    }

    pub(crate) fn get_enabled_gateway_model_by_alias(
        &self,
        alias: &str,
    ) -> Result<Option<GatewayModelRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT gm.id, gm.model_id, gm.display_name, gm.enabled, gm.source,
                    gm.migration_status, gm.legacy_app_type, gm.legacy_source_id,
                    gm.metadata_json, gm.created_at, gm.updated_at
             FROM model_aliases ma
             JOIN gateway_models gm ON gm.id = ma.gateway_model_id
             WHERE ma.alias = ?1 AND gm.enabled = 1 AND gm.migration_status = 'active'
             LIMIT 1",
            [alias],
            |row| {
                Ok(GatewayModelRecord {
                    id: row.get(0)?,
                    model_id: row.get(1)?,
                    display_name: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    source: row.get(4)?,
                    migration_status: row.get(5)?,
                    legacy_app_type: row.get(6)?,
                    legacy_source_id: row.get(7)?,
                    metadata_json: parse_json_column(row.get(8)?, "gateway_models.metadata_json")?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(format!("解析精确模型别名失败: {e}")))
    }

    pub(crate) fn list_enabled_route_targets_with_protocol(
        &self,
        gateway_model_id: &str,
    ) -> Result<Vec<(RouteTargetRecord, String)>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT rt.id, rt.gateway_model_id, rt.upstream_id, rt.target_model,
                        rt.position, rt.enabled, rt.legacy_app_type, rt.legacy_aggregate_id,
                        rt.created_at, rt.updated_at, u.protocol
                 FROM route_targets rt
                 JOIN upstreams u ON u.id = rt.upstream_id
                 WHERE rt.gateway_model_id = ?1 AND rt.enabled = 1 AND u.enabled = 1
                 ORDER BY rt.position ASC, rt.id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取可用路由候选失败: {e}")))?;
        let rows = stmt
            .query_map([gateway_model_id], |row| {
                Ok((
                    RouteTargetRecord {
                        id: row.get(0)?,
                        gateway_model_id: row.get(1)?,
                        upstream_id: row.get(2)?,
                        target_model: row.get(3)?,
                        position: row.get(4)?,
                        enabled: row.get::<_, i64>(5)? != 0,
                        legacy_app_type: row.get(6)?,
                        legacy_aggregate_id: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    },
                    row.get(10)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("读取可用路由候选失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析可用路由候选失败: {e}")))
    }

    pub(crate) fn route_target_is_closed_or_missing(
        &self,
        route_target_id: &str,
    ) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let state = conn
            .query_row(
                "SELECT state FROM route_target_health WHERE route_target_id = ?1",
                [route_target_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| AppError::Database(format!("读取路由健康状态失败: {e}")))?;
        Ok(state.as_deref().is_none_or(|state| state == "closed"))
    }

    pub fn list_model_aliases(&self) -> Result<Vec<ModelAliasRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT alias, gateway_model_id, created_at
                 FROM model_aliases ORDER BY alias ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取模型别名失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ModelAliasRecord {
                    alias: row.get(0)?,
                    gateway_model_id: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取模型别名失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析模型别名失败: {e}")))
    }

    pub fn list_route_targets(&self) -> Result<Vec<RouteTargetRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, gateway_model_id, upstream_id, target_model, position, enabled,
                        legacy_app_type, legacy_aggregate_id, created_at, updated_at
                 FROM route_targets ORDER BY gateway_model_id ASC, position ASC, id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取路由候选失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RouteTargetRecord {
                    id: row.get(0)?,
                    gateway_model_id: row.get(1)?,
                    upstream_id: row.get(2)?,
                    target_model: row.get(3)?,
                    position: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    legacy_app_type: row.get(6)?,
                    legacy_aggregate_id: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取路由候选失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析路由候选失败: {e}")))
    }

    pub fn list_route_target_health(&self) -> Result<Vec<RouteTargetHealthRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT route_target_id, state, consecutive_failures, consecutive_successes,
                        last_success_at, last_failure_at, opened_at, last_error, updated_at
                 FROM route_target_health ORDER BY route_target_id ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取路由健康状态失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RouteTargetHealthRecord {
                    route_target_id: row.get(0)?,
                    state: row.get(1)?,
                    consecutive_failures: row.get(2)?,
                    consecutive_successes: row.get(3)?,
                    last_success_at: row.get(4)?,
                    last_failure_at: row.get(5)?,
                    opened_at: row.get(6)?,
                    last_error: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取路由健康状态失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析路由健康状态失败: {e}")))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn persist_route_target_health(
        &self,
        route_target_id: &str,
        state: &str,
        consecutive_failures: u32,
        consecutive_successes: u32,
        success: Option<bool>,
        error: Option<&str>,
        opened_at: Option<i64>,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        let now = chrono::Utc::now().timestamp_millis();
        let last_success_at = success.is_some_and(|value| value).then_some(now);
        let last_failure_at = success.is_some_and(|value| !value).then_some(now);
        conn.execute(
            "INSERT INTO route_target_health
                (route_target_id, state, consecutive_failures, consecutive_successes,
                 last_success_at, last_failure_at, opened_at, last_error, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(route_target_id) DO UPDATE SET
                state = excluded.state,
                consecutive_failures = excluded.consecutive_failures,
                consecutive_successes = excluded.consecutive_successes,
                last_success_at = COALESCE(excluded.last_success_at, route_target_health.last_success_at),
                last_failure_at = COALESCE(excluded.last_failure_at, route_target_health.last_failure_at),
                opened_at = excluded.opened_at,
                last_error = CASE
                    WHEN excluded.last_success_at IS NOT NULL THEN NULL
                    ELSE COALESCE(excluded.last_error, route_target_health.last_error)
                END,
                updated_at = excluded.updated_at",
            params![
                route_target_id,
                state,
                i64::from(consecutive_failures),
                i64::from(consecutive_successes),
                last_success_at,
                last_failure_at,
                opened_at,
                error,
                now,
            ],
        )
        .map_err(|e| AppError::Database(format!("持久化路由健康状态失败: {e}")))?;
        Ok(())
    }

    pub fn list_gateway_migration_issues(&self) -> Result<Vec<GatewayMigrationIssue>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT migration_key, severity, entity_type, legacy_app_type,
                        legacy_entity_id, code, details_json, created_at
                 FROM gateway_migration_report
                 ORDER BY CASE severity WHEN 'error' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END,
                          entity_type ASC, migration_key ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取迁移报告失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GatewayMigrationIssue {
                    migration_key: row.get(0)?,
                    severity: row.get(1)?,
                    entity_type: row.get(2)?,
                    legacy_app_type: row.get(3)?,
                    legacy_entity_id: row.get(4)?,
                    code: row.get(5)?,
                    details_json: parse_json_column(
                        row.get(6)?,
                        "gateway_migration_report.details_json",
                    )?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取迁移报告失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析迁移报告失败: {e}")))
    }

    pub(crate) fn list_upstream_credentials(
        &self,
        upstream_id: &str,
    ) -> Result<Vec<UpstreamCredentialRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                        key_hint, created_at, updated_at
                 FROM upstream_credentials WHERE upstream_id = ?1
                 ORDER BY credential_kind, id",
            )
            .map_err(|e| AppError::Database(format!("准备读取上游凭据失败: {e}")))?;
        let rows = stmt
            .query_map([upstream_id], |row| {
                Ok(UpstreamCredentialRecord {
                    id: row.get(0)?,
                    upstream_id: row.get(1)?,
                    credential_kind: row.get(2)?,
                    encrypted_payload: row.get(3)?,
                    encryption_scheme: row.get(4)?,
                    key_hint: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取上游凭据失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析上游凭据失败: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    fn seed_routes(db: &Database) {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = db.conn.lock().expect("lock database");
        conn.execute_batch(&format!(
            "INSERT INTO upstreams (id,name,protocol,adapter_type,created_at,updated_at)
             VALUES ('up-1','Upstream','openai_chat','openai',{now},{now});
             INSERT INTO gateway_models (id,model_id,display_name,enabled,source,migration_status,created_at,updated_at)
             VALUES ('gm-1','model','Model',0,'manual','draft',{now},{now});
             INSERT INTO route_targets (id,gateway_model_id,upstream_id,target_model,position,enabled,created_at,updated_at)
             VALUES ('rt-1','gm-1','up-1','vendor-a',0,0,{now},{now}),
                    ('rt-2','gm-1','up-1','vendor-b',1,0,{now},{now});"
        )).expect("seed routes");
    }

    #[test]
    fn config_update_validates_all_safety_boundaries_and_persists_valid_input() {
        let db = Database::memory().expect("memory db");
        let original = db.get_gateway_config_record().expect("read config");
        let mut invalid_configs = Vec::new();

        let mut invalid = original.clone();
        invalid.auth_required = false;
        invalid_configs.push(invalid);
        let mut invalid = original.clone();
        invalid.listen_address = "0.0.0.0".to_string();
        invalid_configs.push(invalid);
        let mut invalid = original.clone();
        invalid.listen_port = 0;
        invalid_configs.push(invalid);
        let clear_timeouts: [fn(&mut GatewayConfigRecord); 4] = [
            |config| config.streaming_first_byte_timeout = 0,
            |config| config.streaming_idle_timeout = 0,
            |config| config.non_streaming_timeout = 0,
            |config| config.circuit_timeout_seconds = 0,
        ];
        for clear_timeout in clear_timeouts {
            let mut invalid = original.clone();
            clear_timeout(&mut invalid);
            invalid_configs.push(invalid);
        }
        let clear_thresholds: [fn(&mut GatewayConfigRecord); 3] = [
            |config| config.circuit_failure_threshold = 0,
            |config| config.circuit_success_threshold = 0,
            |config| config.circuit_min_requests = 0,
        ];
        for clear_threshold in clear_thresholds {
            let mut invalid = original.clone();
            clear_threshold(&mut invalid);
            invalid_configs.push(invalid);
        }
        for error_rate in [f64::NAN, -0.1, 1.1] {
            let mut invalid = original.clone();
            invalid.circuit_error_rate_threshold = error_rate;
            invalid_configs.push(invalid);
        }

        for invalid in invalid_configs {
            assert!(matches!(
                db.update_gateway_config_record(&invalid),
                Err(AppError::InvalidInput(_))
            ));
        }
        assert_eq!(
            db.get_gateway_config_record().expect("unchanged config"),
            original
        );

        let mut updated = original;
        updated.listen_port += 1;
        updated.enable_logging = !updated.enable_logging;
        updated.max_retries += 1;
        db.update_gateway_config_record(&updated)
            .expect("update valid config");
        assert_eq!(
            db.get_gateway_config_record().expect("updated config"),
            updated
        );
    }

    #[test]
    fn model_state_activation_updates_status_and_enabled() {
        let db = Database::memory().expect("memory db");
        let now = chrono::Utc::now().timestamp_millis();
        db.conn
            .lock()
            .expect("lock database")
            .execute_batch(&format!(
                "INSERT INTO upstreams (id,name,enabled,protocol,adapter_type,created_at,updated_at)
                 VALUES ('up-1','Upstream',1,'openai_chat','codex',{now},{now});
                 INSERT INTO gateway_models
                    (id,model_id,display_name,source,migration_status,created_at,updated_at)
                 VALUES ('gm-1','model','Model','manual','conflict',{now},{now});
                 INSERT INTO route_targets
                    (id,gateway_model_id,upstream_id,target_model,position,enabled,created_at,updated_at)
                 VALUES ('rt-1','gm-1','up-1','vendor-model',0,1,{now},{now});"
            ))
            .expect("seed model route");
        assert!(matches!(
            db.set_gateway_model_enabled("gm-1", true),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            db.set_gateway_model_state("gm-1", true, "unknown"),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            db.set_gateway_model_state("gm-1", true, "conflict"),
            Err(AppError::InvalidInput(_))
        ));
        let unchanged = db.list_gateway_models().expect("unchanged model").remove(0);
        assert!(!unchanged.enabled);
        assert_eq!(unchanged.migration_status, "conflict");
        assert!(db
            .set_gateway_model_state("gm-1", true, "active")
            .expect("activate"));
        let model = db.list_gateway_models().expect("list").remove(0);
        assert!(model.enabled);
        assert_eq!(model.migration_status, "active");
    }

    #[test]
    fn reorder_requires_complete_unique_ids_and_commits_order() {
        let db = Database::memory().expect("memory db");
        seed_routes(&db);
        assert!(matches!(
            db.reorder_route_targets("gm-1", &["rt-1".into()]),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            db.reorder_route_targets("gm-1", &["rt-1".into(), "rt-1".into()]),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            db.reorder_route_targets("missing", &[]),
            Err(AppError::InvalidInput(_))
        ));
        assert!(db
            .reorder_route_targets("gm-1", &["rt-2".into(), "missing".into()])
            .is_err());
        let unchanged = db.list_route_targets().expect("unchanged routes");
        assert_eq!(
            (unchanged[0].id.as_str(), unchanged[0].position),
            ("rt-1", 0)
        );
        assert_eq!(
            (unchanged[1].id.as_str(), unchanged[1].position),
            ("rt-2", 1)
        );
        assert!(db
            .set_route_target_enabled("rt-1", true)
            .expect("enable route"));
        assert!(!db
            .set_route_target_enabled("missing", true)
            .expect("missing route"));
        db.reorder_route_targets("gm-1", &["rt-2".into(), "rt-1".into()])
            .expect("reorder");
        let routes = db.list_route_targets().expect("list");
        assert_eq!((routes[0].id.as_str(), routes[0].position), ("rt-2", 0));
        assert_eq!((routes[1].id.as_str(), routes[1].position), ("rt-1", 1));
    }
}
