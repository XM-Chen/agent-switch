/// 故障转移引擎。
///
/// 管理端点切换状态、重试计数、冷却逻辑和回退链。
/// `FailoverState` 提供状态追踪和辅助方法，`proxy_request` 内部驱动路由主循环。
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
    pub fn new(failover_enabled: bool, max_switches: u32, max_retries: u32, test_only: bool) -> Self {
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
            // 测试模式：不增加 switch_count、不加入 failed_ids，仅记录 chain
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

    /// 计算错误对应的冷却时长（秒）。
    ///
    /// - 429：使用 Retry-After 头部 + 1s 缓冲
    /// - 529/503：指数退避（最多 300s）
    /// - 5xx：30-120s
    /// - 401（OAuth）：尝试 refresh 一次；若仍 401，设置永久锁
    fn calculate_cooldown_seconds(&self, error: &ProxyError) -> i64 {
        match error.kind {
            ProxyErrorKind::UpstreamError(429) => {
                // 尝试从 error.message 解析 Retry-After
                let retry_after = parse_retry_after(&error.message).unwrap_or(5);
                retry_after + constants::COOLDOWN_429_RETRY_AFTER_BUFFER_SECS as i64
            }
            ProxyErrorKind::UpstreamError(503) | ProxyErrorKind::UpstreamError(529) => {
                // 指数退避：2^switch_count 秒，最大 300s
                let seconds = 2u64.pow(self.switch_count.min(8));
                seconds.min(constants::COOLDOWN_MAX_EXPONENTIAL_SECS) as i64
            }
            ProxyErrorKind::UpstreamError(code) if (500..=599).contains(&code) => {
                // 5xx：30-120s（按切换次数递增）
                30i64
                    .saturating_add((self.switch_count as i64) * 15)
                    .min(120)
            }
            _ => 30, // 默认冷却
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

fn current_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
