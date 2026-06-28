/// 代理转发错误类型与故障转移决策。
///
/// 定义 `ProxyError`（全链路错误结构）、`ProxyErrorKind`（错误分类）
/// 以及 `should_failover()` 方法，用于判断是否应触发故障转移切换。
/// 参考 prd.md 错误分类规则。
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
    /// 所有候选端点均已尝试但全部失败。
    AllExhausted,
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
            ProxyErrorKind::AllExhausted => 502,
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
    /// - **是**：NetworkError、Timeout、408/429/529、5xx
    /// - **否**：400/405/406/413/414/415/422/501、ProtocolError、AllExhausted、LocalError
    /// - **谨慎**：401/403（由调用方根据端点类型决定）、404
    pub fn should_failover(&self) -> bool {
        match &self.kind {
            ProxyErrorKind::NetworkError | ProxyErrorKind::Timeout => true,
            ProxyErrorKind::UpstreamError(status) => match *status {
                408 | 429 | 529 => true,
                500..=599 => true,
                // 400/405/406/413/414/415/422/501 — 非退避错误
                400 | 405 | 406 | 413 | 414 | 415 | 422 | 501 => false,
                // 401/403/404 — 谨慎处理，默认不退避
                401 | 403 | 404 => false,
                _ => false,
            },
            // 协议错误、已全部耗尽、本地错误不退避
            ProxyErrorKind::ProtocolError
            | ProxyErrorKind::AllExhausted
            | ProxyErrorKind::LocalError => false,
            // AuthError 默认不退避，由调用方判断
            ProxyErrorKind::AuthError => false,
        }
    }

    /// 设置 retryable 标记（用于调用方覆盖默认分类）。
    pub fn retryable(mut self, val: bool) -> Self {
        self.retryable = val;
        self
    }

    /// 设置 stream_started 标记。
    pub fn stream_started(mut self, val: bool) -> Self {
        self.stream_started = val;
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
            ProxyErrorKind::AllExhausted => write!(f, "AllExhausted"),
            ProxyErrorKind::LocalError => write!(f, "LocalError"),
        }
    }
}
