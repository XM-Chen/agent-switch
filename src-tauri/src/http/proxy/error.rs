//! 代理转发错误类型与故障转移决策。
//!
//! 定义 `ProxyError`（全链路错误结构）、`ProxyErrorKind`（错误分类）
//! 以及 `should_failover()` 方法，用于判断是否应触发故障转移切换。
//! 参考 prd.md 错误分类规则。
//!
//! `retryable` 方法为路由主循环预留，保留以对齐设计契约。
#![allow(dead_code)]
use std::fmt;

/// 代理错误分类。
#[derive(Debug, Clone, PartialEq)]
pub enum ProxyErrorKind {
    /// 网络连接失败（DNS 解析失败、连接被拒绝等）。
    NetworkError,
    /// 上游超时（连接超时或响应超时）。
    Timeout,
    /// 上游返回 HTTP 错误（含状态码）。
    UpstreamError(u16),
    /// 协议转换错误（响应格式不符合预期）。
    ProtocolError,
    /// 认证失败（API Key 无效、OAuth token 过期等）。
    AuthError,
    /// 本地错误（DB 错误、配置错误、加密错误等）。
    LocalError,
}

/// 代理错误。
#[derive(Debug, Clone)]
pub struct ProxyError {
    pub kind: ProxyErrorKind,
    pub status: u16,
    pub message: String,
    pub retryable: bool,
    pub stream_started: bool,
}

impl ProxyError {
    /// 创建新的 ProxyError，根据错误分类自动设置默认 HTTP 状态码。
    pub fn new(kind: ProxyErrorKind, message: impl Into<String>) -> Self {
        let status = match &kind {
            ProxyErrorKind::NetworkError => 502,
            ProxyErrorKind::Timeout => 504,
            ProxyErrorKind::UpstreamError(s) => *s,
            ProxyErrorKind::ProtocolError => 500,
            ProxyErrorKind::AuthError => 401,
            ProxyErrorKind::LocalError => 500,
        };
        Self {
            kind,
            status,
            message: message.into(),
            retryable: false,
            stream_started: false,
        }
    }

    /// 判断该错误是否应触发故障转移。
    ///
    /// 规则（参考 prd.md）：
    /// - **是**：NetworkError、Timeout、AuthError、408/429/529、5xx（不含 501）
    /// - **否**：400/405/406/413/414/415/422/501、ProtocolError、LocalError
    /// - **谨慎**：401/403（由调用方根据端点类型决定）、404
    pub fn should_failover(&self) -> bool {
        match &self.kind {
            ProxyErrorKind::NetworkError | ProxyErrorKind::Timeout => true,
            ProxyErrorKind::UpstreamError(status) => match *status {
                408 | 429 | 529 => true,
                // 400/405/406/413/414/415/422/501 — 非退避错误
                // （501 落在 5xx 区间内，必须先于 500..=599 匹配，否则会被误判为可退避）
                400 | 405 | 406 | 413 | 414 | 415 | 422 | 501 => false,
                // 401/403/404 — 谨慎处理，默认不退避
                401 | 403 | 404 => false,
                500..=599 => true,
                _ => false,
            },
            // 协议错误、本地错误不退避
            ProxyErrorKind::ProtocolError | ProxyErrorKind::LocalError => false,
            // AuthError（OAuth 预检刷新失败）：切换到下一个候选端点，
            // 并按 PRD 第 92 行由 calculate_cooldown_seconds 给出 300s 冷却。
            // 不切换会让 selector 反复命中同一已失效凭据端点并触发刷新失败，
            // 放大上游错误率。
            ProxyErrorKind::AuthError => true,
        }
    }

    /// 设置 retryable 标记（用于调用方覆盖默认分类）。
    pub fn retryable(mut self, val: bool) -> Self {
        self.retryable = val;
        self
    }
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} ({:?})", self.status, self.message, self.kind)
    }
}

impl fmt::Display for ProxyErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyErrorKind::NetworkError => write!(f, "NetworkError"),
            ProxyErrorKind::Timeout => write!(f, "Timeout"),
            ProxyErrorKind::UpstreamError(s) => write!(f, "UpstreamError({})", s),
            ProxyErrorKind::ProtocolError => write!(f, "ProtocolError"),
            ProxyErrorKind::AuthError => write!(f, "AuthError"),
            ProxyErrorKind::LocalError => write!(f, "LocalError"),
        }
    }
}
