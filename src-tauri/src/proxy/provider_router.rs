//! 供应商路由器模块
//!
//! 负责选择和管理代理目标供应商，实现智能故障转移

use crate::app_config::AppType;
use crate::database::{AggregateRef, Database};
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};
use crate::proxy::model_mapper::{classify_tier, Tier};
use std::collections::HashMap;
use std::str::FromStr;
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

    /// 选择可用的候选（支持聚合模式 / 故障转移）
    ///
    /// 返回按优先级排序的可用路由候选列表，优先级从高到低：
    /// - **聚合模式开启（D3 互斥接管）**：请求 model 归类 tier → 经 `tier_selection`
    ///   映射到目标聚合 → 经 C2 展平为有序 `(上游, 模型 id)` 候选序列，逐候选按熔断器
    ///   过滤。此分支下当前供应商 / 故障转移队列作为路由轴心暂时失效（配置保留不删）。
    ///   tier 未配置聚合时回退到下面的旧路由（记 warn，不静默失败）。
    /// - 故障转移关闭时：仅返回当前供应商
    /// - 故障转移开启时：仅使用故障转移队列，按队列顺序依次尝试（P1 → P2 → ...）
    ///
    /// `request_model` 用于聚合模式的 tier 归类；非聚合分支忽略此参数。
    pub async fn select_providers(
        &self,
        app_type: &str,
        request_model: &str,
    ) -> Result<Vec<RouteCandidate>, AppError> {
        // ── 聚合模式分支（最高优先级，D3/D5）────────────────────────────
        // 仅在 enabled 时进入；默认关 → 行为与现状逐字节一致。
        if let Some(candidates) = self
            .select_aggregate_candidates(app_type, request_model)
            .await?
        {
            return Ok(candidates);
        }

        let mut result: Vec<RouteCandidate> = Vec::new();
        let mut total_providers = 0usize;
        let mut circuit_open_count = 0usize;

        // 检查该应用的自动故障转移开关是否开启（从 proxy_config 表读取）
        let auto_failover_enabled = match self.db.get_proxy_config_for_app(app_type).await {
            Ok(config) => config.auto_failover_enabled,
            Err(e) => {
                log::error!("[{app_type}] 读取 proxy_config 失败: {e}，默认禁用故障转移");
                false
            }
        };

        if auto_failover_enabled {
            // 故障转移开启：仅按队列顺序依次尝试（P1 → P2 → ...）
            let all_providers = self.db.get_all_providers(app_type)?;

            // 使用 DAO 返回的排序结果，确保和前端展示一致
            let ordered_ids: Vec<String> = self
                .db
                .get_failover_queue(app_type)?
                .into_iter()
                .map(|item| item.provider_id)
                .collect();

            total_providers = ordered_ids.len();

            for provider_id in ordered_ids {
                let Some(provider) = all_providers.get(&provider_id).cloned() else {
                    continue;
                };

                let circuit_key = format!("{app_type}:{}", provider.id);
                let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;

                if breaker.is_available().await {
                    result.push(RouteCandidate {
                        provider,
                        target_model: None,
                    });
                } else {
                    circuit_open_count += 1;
                }
            }
        } else {
            // 故障转移关闭：仅使用当前供应商，跳过熔断器检查
            let current_id = AppType::from_str(app_type)
                .ok()
                .and_then(|app_enum| {
                    crate::settings::get_effective_current_provider(&self.db, &app_enum)
                        .ok()
                        .flatten()
                })
                .or_else(|| self.db.get_current_provider(app_type).ok().flatten());

            if let Some(current_id) = current_id {
                if let Some(current) = self.db.get_provider_by_id(&current_id, app_type)? {
                    total_providers = 1;
                    result.push(RouteCandidate {
                        provider: current,
                        target_model: None,
                    });
                }
            }
        }

        if result.is_empty() {
            if total_providers > 0 && circuit_open_count == total_providers {
                log::warn!("[{app_type}] [FO-004] 所有供应商均已熔断");
                return Err(AppError::AllProvidersCircuitOpen);
            } else {
                log::warn!("[{app_type}] [FO-005] 未配置供应商");
                return Err(AppError::NoProvidersConfigured);
            }
        }

        Ok(result)
    }

    /// 聚合模式候选选择（CC 聚合模式路由 C3，D3/D5）。
    ///
    /// 返回：
    /// - `Ok(None)`：聚合模式未开启，或 tier 未配置目标聚合 → 调用方回退旧路由。
    /// - `Ok(Some(vec))`：聚合模式生效，返回按聚合展平序 + 熔断过滤后的候选。
    /// - `Err(...)`：全部候选熔断（`AllProvidersCircuitOpen`）或聚合为空
    ///   （`NoProvidersConfigured`），复用现有故障转移的错误映射。
    async fn select_aggregate_candidates(
        &self,
        app_type: &str,
        request_model: &str,
    ) -> Result<Option<Vec<RouteCandidate>>, AppError> {
        // C3 仅接管 Claude Code；其它应用即使误写同名配置也必须保持旧路由。
        if app_type != "claude" {
            return Ok(None);
        }

        let config = match self.db.get_cc_aggregate_config(app_type) {
            Ok(config) => config,
            Err(e) => {
                // 读配置失败不能吞掉整个转发路径：记 warn 并退回旧路由。
                log::warn!("[{app_type}] 读取 CC 聚合配置失败: {e}，回退旧路由");
                return Ok(None);
            }
        };

        if !config.enabled {
            return Ok(None);
        }

        // tier 归类复用 model_mapper 的 contains 判据（与 Claude Code 官方分类器一致）。
        let tier = classify_tier(request_model);

        // tier → 目标聚合。fable 未单独配置时降级到 opus 选择，与 map_model 的
        // fable→opus env 兜底方向一致。
        let sel = &config.tier_selection;
        let aggregate_ref: Option<&AggregateRef> = match tier {
            Tier::Fable => sel.fable.as_ref().or(sel.opus.as_ref()),
            Tier::Haiku => sel.haiku.as_ref(),
            Tier::Opus => sel.opus.as_ref(),
            Tier::Sonnet => sel.sonnet.as_ref(),
            Tier::Default => sel.default.as_ref(),
        };

        let Some(aggregate_ref) = aggregate_ref else {
            // tier 无映射：warn + 回退旧路由（不静默失败）。
            log::warn!(
                "[{app_type}] 聚合模式已开启但 tier {tier:?}（model={request_model}）未配置目标聚合，回退旧路由"
            );
            return Ok(None);
        };

        // C2 展平：得到有序 (provider_id, model_id) 候选。
        let flat =
            crate::services::aggregate::flatten_aggregate_ref(&self.db, app_type, aggregate_ref)?;

        if flat.is_empty() {
            // 聚合归零 / 指向已删聚合：候选为空。此时不回退旧路由——聚合模式是
            // 用户显式接管，退回当前供应商会绕过其意图；返回明确错误由客户端感知。
            log::warn!("[{app_type}] 聚合模式 tier {tier:?} 的目标聚合为空（无可路由候选）");
            return Err(AppError::NoProvidersConfigured);
        }

        // 解析 provider_id → Provider，并按熔断器可用性过滤（key 仍 app_type:provider_id，D12）。
        let all_providers = self.db.get_all_providers(app_type)?;
        let total = flat.len();
        let mut circuit_open_count = 0usize;
        let mut candidates: Vec<RouteCandidate> = Vec::with_capacity(flat.len());

        for member in flat {
            let Some(provider) = all_providers.get(&member.provider_id).cloned() else {
                // 队列成员在 providers 表缺失（理论上不应发生，防御性跳过）。
                log::warn!(
                    "[{app_type}] 聚合候选 provider_id={} 在 providers 表中不存在，跳过",
                    member.provider_id
                );
                continue;
            };

            let circuit_key = format!("{app_type}:{}", provider.id);
            let breaker = self.get_or_create_circuit_breaker(&circuit_key).await;
            if breaker.is_available().await {
                candidates.push(RouteCandidate {
                    provider,
                    target_model: Some(member.model_id),
                });
            } else {
                circuit_open_count += 1;
            }
        }

        if candidates.is_empty() {
            if total > 0 && circuit_open_count == total {
                log::warn!("[{app_type}] [FO-004] 聚合候选全部熔断");
                return Err(AppError::AllProvidersCircuitOpen);
            }
            log::warn!("[{app_type}] [FO-005] 聚合候选无可用上游");
            return Err(AppError::NoProvidersConfigured);
        }

        Ok(Some(candidates))
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

    /// 重置熔断器（手动恢复）
    pub async fn reset_circuit_breaker(&self, circuit_key: &str) {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(circuit_key) {
            breaker.reset().await;
        }
    }

    /// 重置指定供应商的熔断器
    pub async fn reset_provider_breaker(&self, provider_id: &str, app_type: &str) {
        let circuit_key = format!("{app_type}:{provider_id}");
        self.reset_circuit_breaker(&circuit_key).await;
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
    use crate::database::{AggregateRef, CcAggregateConfig, Database, TierSelection};
    use crate::services::model_fetch::FetchedModel;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("AGENT_SWITCH_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("AGENT_SWITCH_TEST_HOME", dir.path());
            crate::settings::reload_settings().expect("reload settings");

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("AGENT_SWITCH_TEST_HOME", value),
                None => env::remove_var("AGENT_SWITCH_TEST_HOME"),
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_provider_router_creation() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());
        let router = ProviderRouter::new(db);

        let breaker = router.get_or_create_circuit_breaker("claude:test").await;
        assert!(breaker.allow_request().await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_disabled_uses_current_provider() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider.id, "a");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_uses_queue_order_ignoring_current() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // 设置 sort_index 来控制顺序：b=1, a=2
        let mut provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        provider_a.sort_index = Some(2);
        let mut provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);
        provider_b.sort_index = Some(1);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();

        db.add_to_failover_queue("claude", "b").unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // 启用自动故障转移（使用新的 proxy_config API）
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();

        assert_eq!(providers.len(), 2);
        // 故障转移开启时：仅按队列顺序选择（忽略当前供应商）
        assert_eq!(providers[0].provider.id, "b");
        assert_eq!(providers[1].provider.id, "a");
    }

    #[tokio::test]
    #[serial]
    async fn test_failover_enabled_uses_queue_only_even_if_current_not_in_queue() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let mut provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);
        provider_b.sort_index = Some(1);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();
        db.set_current_provider("claude", "a").unwrap();

        // 只把 b 加入故障转移队列（模拟“当前供应商不在队列里”的常见配置）
        db.add_to_failover_queue("claude", "b").unwrap();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());
        let providers = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider.id, "b");
    }

    #[tokio::test]
    #[serial]
    async fn test_select_providers_does_not_consume_half_open_permit() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        let provider_b =
            Provider::with_id("b".to_string(), "Provider B".to_string(), json!({}), None);

        db.save_provider("claude", &provider_a).unwrap();
        db.save_provider("claude", &provider_b).unwrap();

        db.add_to_failover_queue("claude", "a").unwrap();
        db.add_to_failover_queue("claude", "b").unwrap();

        // 启用自动故障转移（使用新的 proxy_config API）
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        router
            .record_result("b", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        let providers = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();
        assert_eq!(providers.len(), 2);

        assert!(router.allow_provider_request("b", "claude").await.allowed);
    }

    #[tokio::test]
    #[serial]
    async fn test_release_permit_neutral_frees_half_open_slot() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // 配置熔断器：1 次失败即熔断，0 秒超时立即进入 HalfOpen
        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 0,
            ..Default::default()
        })
        .await
        .unwrap();

        let provider_a =
            Provider::with_id("a".to_string(), "Provider A".to_string(), json!({}), None);
        db.save_provider("claude", &provider_a).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // 启用自动故障转移
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let router = ProviderRouter::new(db.clone());

        // 触发熔断：1 次失败
        router
            .record_result("a", "claude", false, false, Some("fail".to_string()))
            .await
            .unwrap();

        // 第一次请求：获取 HalfOpen 探测名额
        let first = router.allow_provider_request("a", "claude").await;
        assert!(first.allowed);
        assert!(first.used_half_open_permit);

        // 第二次请求应被拒绝（名额已被占用）
        let second = router.allow_provider_request("a", "claude").await;
        assert!(!second.allowed);

        // 使用 release_permit_neutral 释放名额（不影响健康统计）
        router
            .release_permit_neutral("a", "claude", first.used_half_open_permit)
            .await;

        // 第三次请求应被允许（名额已释放）
        let third = router.allow_provider_request("a", "claude").await;
        assert!(third.allowed);
        assert!(third.used_half_open_permit);
    }

    // ===== 聚合模式路由测试（CC 聚合 C3，D3/D5）=====

    /// 保存 provider 并加入故障转移队列（聚合候选来源严格 = 队列成员，D4/D5）。
    async fn seed_queue_provider(db: &Database, id: &str, sort_index: usize) {
        let mut p = Provider::with_id(id.to_string(), id.to_string(), json!({}), None);
        p.sort_index = Some(sort_index);
        db.save_provider("claude", &p).unwrap();
        db.add_to_failover_queue("claude", id).unwrap();
    }

    fn fetch_model(db: &Database, provider_id: &str, model_id: &str) {
        db.replace_fetched_models(
            "claude",
            provider_id,
            &[FetchedModel {
                id: model_id.to_string(),
                owned_by: None,
            }],
            100,
        )
        .unwrap();
    }

    fn enable_aggregate(db: &Database, tier_selection: TierSelection) {
        db.set_cc_aggregate_config(
            "claude",
            &CcAggregateConfig {
                enabled: true,
                tier_selection,
            },
        )
        .unwrap();
    }

    // 开启聚合模式：sonnet 请求 → tier_selection → 自动聚合 → 展平候选，
    // 路由到队列内首个可用上游并携带正确的 target_model（改写目标）。
    #[tokio::test]
    #[serial]
    async fn aggregate_sonnet_routes_to_auto_aggregate_candidates() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // p1、p2 都提供 glm-4.6（大小写归一同一聚合），队列序 p1→p2。
        seed_queue_provider(&db, "p1", 0).await;
        seed_queue_provider(&db, "p2", 1).await;
        fetch_model(&db, "p1", "glm-4.6");
        fetch_model(&db, "p2", "glm-4.6");

        enable_aggregate(
            &db,
            TierSelection {
                sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        let candidates = router
            .select_providers("claude", "claude-sonnet-4-5-20250929")
            .await
            .unwrap();

        assert_eq!(candidates.len(), 2, "聚合展平应含两个上游候选");
        // 队列序：p1 在前。
        assert_eq!(candidates[0].provider.id, "p1");
        assert_eq!(candidates[0].target_model.as_deref(), Some("glm-4.6"));
        assert_eq!(candidates[1].provider.id, "p2");
        assert_eq!(candidates[1].target_model.as_deref(), Some("glm-4.6"));
    }

    // tier_selection 指向自定义聚合（opus→CCC=[C,D]）：按「外层成员序 × 内层上游序」
    // 展平逐个尝试（R16/D7）。
    #[tokio::test]
    #[serial]
    async fn aggregate_opus_routes_to_custom_aggregate_flatten_order() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        seed_queue_provider(&db, "p2", 0).await;
        seed_queue_provider(&db, "p3", 1).await;
        // C 由 p2、p3 提供；D 只由 p3 提供。
        fetch_model(&db, "p2", "C");
        db.replace_fetched_models(
            "claude",
            "p3",
            &[
                FetchedModel {
                    id: "C".into(),
                    owned_by: None,
                },
                FetchedModel {
                    id: "D".into(),
                    owned_by: None,
                },
            ],
            100,
        )
        .unwrap();

        let ccc = db
            .create_custom_aggregate("claude", "CCC", &["C".into(), "D".into()])
            .unwrap();
        enable_aggregate(
            &db,
            TierSelection {
                opus: Some(AggregateRef::Custom(ccc)),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        let candidates = router
            .select_providers("claude", "claude-opus-4-5")
            .await
            .unwrap();

        let seq: Vec<(String, Option<String>)> = candidates
            .iter()
            .map(|c| (c.provider.id.clone(), c.target_model.clone()))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("p2".into(), Some("C".into())),
                ("p3".into(), Some("C".into())),
                ("p3".into(), Some("D".into())),
            ],
            "外层成员序 × 内层上游序"
        );
    }

    // 首选上游熔断时，聚合候选序列跳过熔断上游，故障转移到下一个可用上游。
    #[tokio::test]
    #[serial]
    async fn aggregate_skips_circuit_open_candidate() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // 1 次失败即熔断。
        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 60,
            ..Default::default()
        })
        .await
        .unwrap();

        seed_queue_provider(&db, "p1", 0).await;
        seed_queue_provider(&db, "p2", 1).await;
        fetch_model(&db, "p1", "glm-4.6");
        fetch_model(&db, "p2", "glm-4.6");

        enable_aggregate(
            &db,
            TierSelection {
                sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        // 熔断 p1。
        router
            .record_result("p1", "claude", false, false, Some("fail".into()))
            .await
            .unwrap();

        let candidates = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1, "p1 已熔断，只剩 p2");
        assert_eq!(candidates[0].provider.id, "p2");
        assert_eq!(candidates[0].target_model.as_deref(), Some("glm-4.6"));
    }

    // 候选全部熔断 → 返回 AllProvidersCircuitOpen（复用现有故障转移错误映射）。
    #[tokio::test]
    #[serial]
    async fn aggregate_all_candidates_circuit_open_errors() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        db.update_circuit_breaker_config(&CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_seconds: 60,
            ..Default::default()
        })
        .await
        .unwrap();

        seed_queue_provider(&db, "p1", 0).await;
        fetch_model(&db, "p1", "glm-4.6");

        enable_aggregate(
            &db,
            TierSelection {
                sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        router
            .record_result("p1", "claude", false, false, Some("fail".into()))
            .await
            .unwrap();

        let err = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AllProvidersCircuitOpen));
    }

    // tier 无映射：warn + 回退旧路由（当前供应商），不静默失败。
    #[tokio::test]
    #[serial]
    async fn aggregate_tier_without_mapping_falls_back_to_current_provider() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        // 队列/聚合仅配了 sonnet；opus 请求无映射 → 回退旧路由。
        seed_queue_provider(&db, "p1", 0).await;
        fetch_model(&db, "p1", "glm-4.6");

        // 另设当前供应商 cur（不在聚合内），用于验证回退。
        let cur = Provider::with_id("cur".into(), "cur".into(), json!({}), None);
        db.save_provider("claude", &cur).unwrap();
        db.set_current_provider("claude", "cur").unwrap();

        enable_aggregate(
            &db,
            TierSelection {
                sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        let candidates = router
            .select_providers("claude", "claude-opus-4-5")
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1, "opus 无映射 → 回退当前供应商");
        assert_eq!(candidates[0].provider.id, "cur");
        assert!(
            candidates[0].target_model.is_none(),
            "回退路径候选不带 target_model"
        );
    }

    // 关闭聚合模式后路由完全退化为原 ccs 行为（当前供应商），旧配置无损。
    #[tokio::test]
    #[serial]
    async fn aggregate_disabled_restores_current_provider_routing() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        seed_queue_provider(&db, "p1", 0).await;
        fetch_model(&db, "p1", "glm-4.6");
        let cur = Provider::with_id("cur".into(), "cur".into(), json!({}), None);
        db.save_provider("claude", &cur).unwrap();
        db.set_current_provider("claude", "cur").unwrap();

        // 配置存在但 enabled=false（保留 tier_selection，D3 关闭即恢复）。
        db.set_cc_aggregate_config(
            "claude",
            &CcAggregateConfig {
                enabled: false,
                tier_selection: TierSelection {
                    sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                    ..Default::default()
                },
            },
        )
        .unwrap();

        let router = ProviderRouter::new(db.clone());
        let candidates = router
            .select_providers("claude", "claude-sonnet-4-5")
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider.id, "cur", "聚合关 → 当前供应商");
        assert!(candidates[0].target_model.is_none());
    }

    // fable 未单独配置时降级到 opus 选择（与 map_model 的 fable→opus 兜底方向一致）。
    #[tokio::test]
    #[serial]
    async fn aggregate_fable_falls_back_to_opus_selection() {
        let _home = TempHome::new();
        let db = Arc::new(Database::memory().unwrap());

        seed_queue_provider(&db, "p1", 0).await;
        fetch_model(&db, "p1", "opus-agg");

        enable_aggregate(
            &db,
            TierSelection {
                // 只配 opus，不配 fable。
                opus: Some(AggregateRef::Auto("opus-agg".into())),
                ..Default::default()
            },
        );

        let router = ProviderRouter::new(db.clone());
        let candidates = router
            .select_providers("claude", "claude-fable-5")
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].provider.id, "p1");
        assert_eq!(
            candidates[0].target_model.as_deref(),
            Some("opus-agg"),
            "fable 未配置 → 用 opus 聚合选择"
        );
    }
}
