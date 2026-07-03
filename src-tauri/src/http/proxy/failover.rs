//! 故障转移引擎。
//!
//! 管理端点切换状态、重试计数、冷却逻辑和回退链。
//! `FailoverState` 提供状态追踪和辅助方法，`proxy_request` 内部驱动路由主循环。
//!
//! `RouteAttempt` 与部分重试计数辅助方法为路由管理层预留，尚未在主循环全量接线。
#![allow(dead_code)]
use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::db::dao::endpoints::EndpointRow;
use crate::http::proxy::constants;
use crate::http::proxy::error::{ProxyError, ProxyErrorKind};

/// 回退跳转记录。
#[derive(Debug, Clone, Serialize)]
pub struct FallbackHop {
    /// 端点 ID。
    pub endpoint_id: String,
    /// 模型名称。
    pub model: Option<String>,
    /// 状态（"success" / "failed" / "skipped"）。
    pub status: String,
    /// 错误信息。
    pub error_message: Option<String>,
    /// 延迟（毫秒）。
    pub latency_ms: Option<u64>,
}

/// 路由尝试结果。
#[derive(Debug)]
pub enum RouteAttempt<T> {
    /// 成功。
    Success(T),
    /// 可重试错误。
    Retryable(ProxyError),
    /// 不可重试错误（立即终止）。
    Fatal(ProxyError),
}

/// 故障转移状态。
pub struct FailoverState {
    /// 当前切换次数。
    pub switch_count: u32,
    /// 最大切换次数（<=1 表示不启用故障转移）。
    pub max_switches: u32,
    /// 已失败的端点 ID 集合。
    pub failed_ids: HashSet<String>,
    /// 每个端点的重试计数。
    pub retry_counts: HashMap<String, u32>,
    /// 同端点最大重试次数。
    pub max_retries: u32,
    /// 路由级冷却倍率。仅用于通用上游/网络类冷却；AuthError 与 Retry-After 保持协议/PRD 固定语义。
    pub cooldown_multiplier: f64,
    /// 最后一次错误。
    pub last_error: Option<ProxyError>,
    /// 是否已开始向客户端发送流数据。
    pub stream_started: bool,
    /// 回退链记录。
    pub chain: Vec<FallbackHop>,
    /// 请求开始时间（Unix 毫秒）。
    pub start_time_ms: u64,
    /// 是否为测试模式（不写冷却/故障转移状态）。
    pub test_only: bool,
}

impl FailoverState {
    /// 创建故障转移状态。
    ///
    /// - `test_only`：为 true 时进入测试模式，`record_failure` 不产生冷却值（返回 0），
    ///   并且允许遍历所有端点（不受 max_switches 限制）。
    pub fn new(
        failover_enabled: bool,
        max_switches: u32,
        max_retries: u32,
        cooldown_multiplier: f64,
        test_only: bool,
    ) -> Self {
        Self {
            switch_count: 0,
            max_switches: if test_only {
                // 测试模式：允许遍历所有端点
                999
            } else if failover_enabled {
                max_switches
            } else {
                1
            },
            failed_ids: HashSet::new(),
            retry_counts: HashMap::new(),
            max_retries,
            cooldown_multiplier: sanitize_cooldown_multiplier(cooldown_multiplier),
            last_error: None,
            stream_started: false,
            chain: Vec::new(),
            start_time_ms: current_time_millis(),
            test_only,
        }
    }

    /// 检查是否还可以继续尝试下一个端点。
    pub fn can_continue(&self) -> bool {
        !self.stream_started && self.switch_count < self.max_switches
    }

    /// 记录端点失败（增 switch_count、记录 chain、标记冷却）。
    ///
    /// 返回此端点的通用冷却时长（秒），供调用方设置 `cooldown_until`。
    /// 测试模式下始终返回 0（不产生冷却）。
    pub fn record_failure(
        &mut self,
        endpoint: &EndpointRow,
        error: &ProxyError,
        latency_ms: u64,
    ) -> i64 {
        if self.test_only {
            // 测试模式：不增加 switch_count、不设冷却，但必须加入 failed_ids
            // 防止 selector 重复选中同一端点导致无限循环。
            self.failed_ids.insert(endpoint.id.clone());
            self.last_error = Some(error.clone());
            self.chain.push(FallbackHop {
                endpoint_id: endpoint.id.clone(),
                model: None,
                status: "failed".to_string(),
                error_message: Some(error.message.clone()),
                latency_ms: Some(latency_ms),
            });
            return 0;
        }

        self.switch_count += 1;
        self.failed_ids.insert(endpoint.id.clone());
        self.last_error = Some(error.clone());

        // 记录回退链
        self.chain.push(FallbackHop {
            endpoint_id: endpoint.id.clone(),
            model: None,
            status: "failed".to_string(),
            error_message: Some(error.message.clone()),
            latency_ms: Some(latency_ms),
        });

        // 计算冷却时长
        self.calculate_cooldown_seconds(error)
    }

    /// 记录端点成功（写入 chain）。
    pub fn record_success(&mut self, endpoint: &EndpointRow, latency_ms: u64) {
        self.chain.push(FallbackHop {
            endpoint_id: endpoint.id.clone(),
            model: None,
            status: "success".to_string(),
            error_message: None,
            latency_ms: Some(latency_ms),
        });
    }

    /// 获取当前端点的重试次数并递增。
    pub fn get_and_increment_retry(&mut self, endpoint_id: &str) -> u32 {
        let count = self
            .retry_counts
            .entry(endpoint_id.to_string())
            .or_insert(0);
        *count += 1;
        *count
    }

    /// 判断端点是否达到最大重试次数。
    pub fn is_max_retries_reached(&self, endpoint_id: &str) -> bool {
        self.retry_counts.get(endpoint_id).copied().unwrap_or(0) >= self.max_retries
    }

    /// 检查是否需要重试（可重试且未超限）。
    pub fn should_retry(&self, endpoint_id: &str, error: &ProxyError) -> bool {
        error.retryable && !error.stream_started && !self.is_max_retries_reached(endpoint_id)
    }

    /// 同端点重试前应等待的延迟。
    ///
    /// PRD R3.3 / route policy 指定 retryDelay=500ms；可重试错误（429/5xx 等）
    /// 返回后立即重发会 hammer 上游，故同端点下一次重试前等待此 `Duration`。
    pub fn same_account_retry_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(constants::SAME_ACCOUNT_RETRY_DELAY_MS)
    }

    /// 计算错误对应的冷却时长（秒）。
    ///
    /// 契约参考 `.trellis/spec/guides/app-stack-conventions.md` §10.1 与 PRD 第 92 行。
    /// - 429：使用 Retry-After 头部 + 1s 缓冲（最小 1s，上限 300s）。
    /// - 529/503：指数退避（最多 300s）。
    /// - 5xx（其余）：30-120s（按切换次数递增）。
    /// - AuthError（OAuth 预检刷新失败）：**300s**（PRD 第 92 行，30s 会被 selector 反复命中失效端点放大错误率）。
    /// - NetworkError / Timeout：固定基准（30s / 60s）。
    /// - 其余未分类瞬时错误：30s 默认。
    ///
    /// `cooldown_multiplier` 仅作用于上游通用类（429/5xx/503/529/网络/超时/默认），
    /// **不**作用于 AuthError 与 429 的 Retry-After（这两类由 PRD/上游协议语义固定）。
    fn calculate_cooldown_seconds(&self, error: &ProxyError) -> i64 {
        match error.kind {
            ProxyErrorKind::UpstreamError(429) => {
                // Retry-After 由上游协议语义决定，不受 cooldown_multiplier 影响。
                let retry_after = parse_retry_after(&error.message)
                    .unwrap_or(constants::COOLDOWN_429_DEFAULT_RETRY_AFTER_SECS);
                let secs = retry_after + constants::COOLDOWN_429_RETRY_AFTER_BUFFER_SECS;
                secs.clamp(
                    constants::COOLDOWN_429_MIN_SECS,
                    constants::COOLDOWN_429_CAP_SECS,
                )
            }
            ProxyErrorKind::UpstreamError(503) | ProxyErrorKind::UpstreamError(529) => {
                // 指数退避：2^switch_count 秒，最大 300s。
                let power = self.switch_count.min(constants::COOLDOWN_EXPONENTIAL_MAX_POWER);
                let base = 2u64.pow(power) as i64;
                apply_multiplier(
                    base,
                    self.cooldown_multiplier,
                    constants::COOLDOWN_MAX_EXPONENTIAL_SECS as i64,
                )
            }
            ProxyErrorKind::UpstreamError(code) if (500..=599).contains(&code) => {
                // 5xx：30-120s（按切换次数递增）。
                let base = constants::COOLDOWN_5XX_BASE_SECS
                    .saturating_add((self.switch_count as i64) * constants::COOLDOWN_5XX_STEP_SECS);
                apply_multiplier(
                    base,
                    self.cooldown_multiplier,
                    constants::COOLDOWN_5XX_CAP_SECS,
                )
            }
            // AuthError：PRD 第 92 行固定 300s，不受 cooldown_multiplier 影响，
            // 避免 UI 把认证冷却误调到极短值反复触发刷新失败。
            ProxyErrorKind::AuthError => constants::COOLDOWN_AUTH_ERROR_SECS,
            ProxyErrorKind::NetworkError => apply_multiplier(
                constants::COOLDOWN_NETWORK_ERROR_SECS,
                self.cooldown_multiplier,
                constants::COOLDOWN_5XX_CAP_SECS,
            ),
            ProxyErrorKind::Timeout => apply_multiplier(
                constants::COOLDOWN_TIMEOUT_SECS,
                self.cooldown_multiplier,
                constants::COOLDOWN_5XX_CAP_SECS,
            ),
            // 其余未分类瞬时错误：默认 30s。新增错误类型必须在上面显式分类，
            // 禁止依赖此默认分支（参考 spec §10.1）。
            _ => apply_multiplier(
                constants::COOLDOWN_DEFAULT_SECS,
                self.cooldown_multiplier,
                constants::COOLDOWN_5XX_CAP_SECS,
            ),
        }
    }

    /// 构建回退链的 JSON 字符串。
    pub fn chain_to_json(&self) -> String {
        serde_json::to_string(&self.chain).unwrap_or_else(|_| "[]".to_string())
    }
}

/// 从错误消息中解析 Retry-After 值（秒）。
fn parse_retry_after(message: &str) -> Option<i64> {
    // 尝试从 JSON 中提取 retry_after 字段
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(message) {
        if let Some(ra) = value.get("retry_after").and_then(|v| v.as_i64()) {
            return Some(ra);
        }
        if let Some(ra) = value
            .get("error")
            .and_then(|e| e.get("retry_after").and_then(|v| v.as_i64()))
        {
            return Some(ra);
        }
    }
    // 尝试从纯文本中解析数字
    if let Ok(n) = message.trim().parse::<i64>() {
        return Some(n);
    }
    None
}

fn apply_multiplier(base_secs: i64, multiplier: f64, cap_secs: i64) -> i64 {
    let base = base_secs.max(1) as f64;
    let scaled = (base * sanitize_cooldown_multiplier(multiplier)).ceil() as i64;
    scaled.max(1).min(cap_secs.max(1))
}

fn sanitize_cooldown_multiplier(multiplier: f64) -> f64 {
    if multiplier.is_finite() && multiplier > 0.0 {
        multiplier
    } else {
        1.0
    }
}

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
