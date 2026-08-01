//! 供应商路由器模块
//!
//! 负责选择和管理代理目标供应商，实现智能故障转移

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::gateway::core::{IngressProtocol, RouteResolutionError};
use crate::provider::Provider;
use crate::proxy::circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 路由层候选（CC 聚合模式路由 C3）。
///
/// 每个候选携带**已解析的 Provider 对象**（供 forwarder 直接转发）以及聚合模式下
/// 要改写成的目标上游模型 id（`target_model`）。
///
/// 命名说明（与 C2 的类型区分，避免静默撞名）：
/// - `crate::services::aggregate::RouteCandidate { provider_id, model_id }` 是 C2 的
///   **数据层**展平候选，只含字符串 id，来自 DB 派生。
/// - 本类型是 C3 的**路由层**候选，含完整 `Provider` 与 forwarder 需要的改写目标。
///   两者语义不同、字段不同，故各自保留，本层在消费 C2 结果时按 provider_id 解析出
///   `Provider` 并包成本类型。
#[derive(Debug, Clone)]
pub struct RouteCandidate {
    /// 稳定路由候选 ID；阶段 3 熔断/健康以它为 key。
    /// 旧路由测试构造的兼容候选可为 None，生产模型路由始终为 Some。
    pub route_target_id: Option<String>,
    /// 新影子域的 upstream ID；生产模型路由始终为 Some。
    pub upstream_id: Option<String>,
    /// 上游 adapter 身份。数据面必须按它选择 adapter，不能复用入站 AppType。
    pub adapter_app_type: AppType,
    /// 选中的供应商（故障转移链中的一环）。
    pub provider: Provider,
    /// 聚合模式下要改写成的上游模型 id（精确聚合 key）。
    ///
    /// `None` = 非聚合路由（当前供应商 / 故障转移队列），沿用现有 `model_mapper` 逻辑。
    /// `Some(id)` = 聚合路由，forward 前把请求体 model 改写为该 id 并跳过 env 二次映射。
    pub target_model: Option<String>,
}

/// 供应商路由器
pub struct ProviderRouter {
    /// 数据库连接
    db: Arc<Database>,
    /// 熔断器管理器 - key 格式: "app_type:provider_id"
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
}

impl ProviderRouter {
    /// 创建新的供应商路由器
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn select_gateway_routes(
        &self,
        ingress: IngressProtocol,
        request_model: &str,
    ) -> Result<(String, Vec<RouteCandidate>), RouteResolutionError> {
        let (gateway_model, targets) =
            crate::gateway::application::resolve_database_route(&self.db, ingress, request_model)?;
        let mut candidates = Vec::with_capacity(targets.len());
        for target in targets {
            let Some(upstream) = self.db.get_upstream(&target.upstream_id).ok().flatten() else {
                continue;
            };
            let (adapter_app_type, provider) =
                match crate::gateway::infrastructure::load_upstream_provider(&self.db, &upstream) {
                    Ok(projected) => projected,
                    Err(error) => {
                        log::warn!(
                            "跳过运行时不可用的上游: upstream_id={}, error={error}",
                            upstream.id
                        );
                        continue;
                    }
                };
            candidates.push(RouteCandidate {
                route_target_id: Some(target.route_target_id),
                upstream_id: Some(target.upstream_id),
                adapter_app_type,
                provider,
                target_model: Some(target.target_model),
            });
        }
        if candidates.is_empty() {
            return Err(RouteResolutionError::NoAvailableTarget {
                gateway_model_id: request_model.to_string(),
            });
        }
        Ok((gateway_model.gateway_model_id, candidates))
    }

    pub async fn allow_route_target_request(&self, route_target_id: &str) -> AllowResult {
        let breaker = self.get_or_create_circuit_breaker(route_target_id).await;
        breaker.allow_request().await
    }

    pub async fn record_route_target_result(
        &self,
        route_target_id: &str,
        used_half_open_permit: bool,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), AppError> {
        let breaker = self.get_or_create_circuit_breaker(route_target_id).await;
        if success {
            breaker.record_success(used_half_open_permit).await;
        } else {
            breaker.record_failure(used_half_open_permit).await;
        }
        let stats = breaker.get_stats().await;
        let state = breaker.get_state().await;
        let opened_at = matches!(state, crate::proxy::circuit_breaker::CircuitState::Open)
            .then(|| chrono::Utc::now().timestamp_millis());
        self.db.persist_route_target_health(
            route_target_id,
            &state.to_string(),
            stats.consecutive_failures,
            stats.consecutive_successes,
            Some(success),
            error_msg.as_deref(),
            opened_at,
        )?;
        Ok(())
    }

    pub async fn release_route_target_permit_neutral(
        &self,
        route_target_id: &str,
        used_half_open_permit: bool,
    ) {
        if !used_half_open_permit {
            return;
        }
        let breaker = self.get_or_create_circuit_breaker(route_target_id).await;
        breaker.release_half_open_permit();
    }

    /// 请求执行前获取熔断器“放行许可”
    ///
    /// - Closed：直接放行
    /// - Open：超时到达后切到 HalfOpen 并放行一次探测
    /// - HalfOpen：按限流规则放行探测
    ///
    /// 注意：调用方必须在请求结束后通过 `record_result()` 释放 HalfOpen 名额，
    /// 否则会导致该 Provider 长时间无法进入探测状态。
    pub async fn allow_provider_request(&self, provider_id: &str, app_type: &str) -> AllowResult {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.allow_request().await
    }

    /// 记录供应商请求结果
    pub async fn record_result(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
        success: bool,
        error_msg: Option<String>,
    ) -> Result<(), AppError> {
        // 1. 按应用独立获取熔断器配置
        let failure_threshold = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => app_config.circuit_failure_threshold,
            Err(_) => 5, // 默认值
        };

        // 2. 更新熔断器状态
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;

        if success {
            breaker.record_success(used_half_open_permit).await;
        } else {
            breaker.record_failure(used_half_open_permit).await;
        }

        // 3. 更新数据库健康状态（使用配置的阈值）
        self.db
            .update_provider_health_with_threshold(
                provider_id,
                app_type,
                success,
                error_msg.clone(),
                failure_threshold,
            )
            .await?;

        Ok(())
    }

    /// 仅释放 HalfOpen permit，不影响健康统计（neutral 接口）
    ///
    /// 用于整流器等场景：请求结果不应计入 Provider 健康度，
    /// 但仍需释放占用的探测名额，避免 HalfOpen 状态卡死
    pub async fn release_permit_neutral(
        &self,
        provider_id: &str,
        app_type: &str,
        used_half_open_permit: bool,
    ) {
        if !used_half_open_permit {
            return;
        }
        let circuit_key = format!("{app_type}:{provider_id}");
        let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
        breaker.release_half_open_permit();
    }

    /// 更新所有熔断器的配置（热更新）
    pub async fn update_all_configs(&self, config: CircuitBreakerConfig) {
        let breakers = self.circuit_breakers.read().await;
        for breaker in breakers.values() {
            breaker.update_config(config.clone()).await;
        }
    }

    /// 更新指定应用已创建熔断器的配置（热更新）
    pub async fn update_app_configs(&self, app_type: &str, config: CircuitBreakerConfig) {
        let prefix = format!("{app_type}:");
        let breakers = self.circuit_breakers.read().await;
        for (key, breaker) in breakers.iter() {
            if key.starts_with(&prefix) {
                breaker.update_config(config.clone()).await;
            }
        }
    }

    /// 获取熔断器状态
    #[allow(dead_code)]
    pub async fn get_circuit_breaker_stats(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Option<crate::proxy::circuit_breaker::CircuitBreakerStats> {
        let circuit_key = format!("{app_type}:{provider_id}");
        let breakers = self.circuit_breakers.read().await;

        if let Some(breaker) = breakers.get(&circuit_key) {
            Some(breaker.get_stats().await)
        } else {
            None
        }
    }

    /// 获取或创建熔断器
    async fn get_or_create_circuit_breaker(&self, key: &str) -> Arc<CircuitBreaker> {
        // 先尝试读锁获取
        {
            let breakers = self.circuit_breakers.read().await;
            if let Some(breaker) = breakers.get(key) {
                return breaker.clone();
            }
        }

        // 如果不存在，获取写锁创建
        let mut breakers = self.circuit_breakers.write().await;

        // 双重检查，防止竞争条件
        if let Some(breaker) = breakers.get(key) {
            return breaker.clone();
        }

        // 从 key 中提取 app_type (格式: "app_type:provider_id")
        let app_type = key.split(':').next().unwrap_or("claude");

        // 按应用独立读取熔断器配置
        let config = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(app_config) => crate::proxy::circuit_breaker::CircuitBreakerConfig {
                failure_threshold: app_config.circuit_failure_threshold,
                success_threshold: app_config.circuit_success_threshold,
                timeout_seconds: app_config.circuit_timeout_seconds as u64,
                error_rate_threshold: app_config.circuit_error_rate_threshold,
                min_requests: app_config.circuit_min_requests,
            },
            Err(_) => crate::proxy::circuit_breaker::CircuitBreakerConfig::default(),
        };

        let breaker = Arc::new(CircuitBreaker::new(config));
        breakers.insert(key.to_string(), breaker.clone());

        breaker
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
    use rusqlite::params;

    #[tokio::test]
    async fn gateway_route_health_is_keyed_by_route_target_id() {
        let db = Arc::new(Database::memory().unwrap());
        let now = chrono::Utc::now().timestamp_millis();
        {
            let conn = db.conn.lock().expect("lock");
            conn.execute_batch(&format!(
                "INSERT INTO upstreams
                    (id, name, enabled, protocol, adapter_type, base_url, config_json, created_at, updated_at)
                 VALUES ('up-1', 'Upstream', 1, 'anthropic', 'claude', 'https://up.invalid',
                         '{{\"legacySettings\":{{}},\"legacyMeta\":{{}}}}', {now}, {now});
                 INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status, created_at, updated_at)
                 VALUES ('gm-1', 'stable-model', 'Stable', 1, 'manual', 'active', {now}, {now});
                 INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
                 VALUES ('target-1', 'gm-1', 'up-1', 'vendor-model', 0, 1, {now}, {now});"
            ))
            .expect("seed route target");
            let protector = PlatformCredentialProtector;
            let encrypted = protector
                .protect(b"runtime-secret")
                .expect("protect credential");
            conn.execute(
                "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     created_at, updated_at)
                 VALUES ('cred-1', 'up-1', 'x_api_key', ?1, ?2, ?3, ?3)",
                params![encrypted, protector.scheme(), now],
            )
            .expect("seed credential");
        }

        let router = ProviderRouter::new(db.clone());
        let (gateway_model_id, candidates) = router
            .select_gateway_routes(IngressProtocol::AnthropicMessages, "stable-model")
            .await
            .expect("select route");
        assert_eq!(gateway_model_id, "gm-1");
        assert_eq!(candidates[0].route_target_id.as_deref(), Some("target-1"));
        assert_eq!(candidates[0].adapter_app_type, AppType::Claude);
        assert_eq!(
            candidates[0]
                .provider
                .settings_config
                .pointer("/env/ANTHROPIC_API_KEY")
                .and_then(serde_json::Value::as_str),
            Some("runtime-secret")
        );

        for _ in 0..4 {
            router
                .record_route_target_result("target-1", false, false, Some("boom".into()))
                .await
                .expect("record failure");
        }
        let health = db.list_route_target_health().expect("read health");
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].route_target_id, "target-1");
        assert_eq!(health[0].state, "open");
        assert!(matches!(
            router
                .select_gateway_routes(IngressProtocol::AnthropicMessages, "stable-model")
                .await,
            Err(RouteResolutionError::NoAvailableTarget { .. })
        ));
    }

    #[tokio::test]
    async fn gateway_route_without_runtime_credential_is_not_forwardable() {
        let db = Arc::new(Database::memory().unwrap());
        let now = chrono::Utc::now().timestamp_millis();
        db.conn
            .lock()
            .expect("lock")
            .execute_batch(&format!(
                "INSERT INTO upstreams
                    (id, name, enabled, protocol, adapter_type, base_url, config_json, created_at, updated_at)
                 VALUES ('up-missing', 'Missing', 1, 'openai_responses', 'codex',
                         'https://up.invalid', '{{\"legacySettings\":{{}},\"legacyMeta\":{{}}}}', {now}, {now});
                 INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status, created_at, updated_at)
                 VALUES ('gm-missing', 'missing-secret-model', 'Missing', 1, 'manual', 'active', {now}, {now});
                 INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
                 VALUES ('target-missing', 'gm-missing', 'up-missing', 'vendor-model', 0, 1, {now}, {now});"
            ))
            .expect("seed missing credential route");

        let router = ProviderRouter::new(db);
        assert!(matches!(
            router
                .select_gateway_routes(IngressProtocol::OpenAiResponses, "missing-secret-model")
                .await,
            Err(RouteResolutionError::NoAvailableTarget { .. })
        ));
    }
}
