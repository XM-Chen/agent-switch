/// 流首块缓冲守卫。
///
/// 在流式响应中缓冲第一个 chunk，检测上游是否返回了错误。
/// 若首块为错误，在向客户端发送任何 SSE 数据之前返回 `ProxyError`，
/// 从而允许故障转移引擎切换端点。
///
/// 一旦首块确认正常，标记 `stream_started = true`，此后禁止切换。
use std::pin::Pin;

use axum::http::StatusCode;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};

use crate::http::proxy::error::{ProxyError, ProxyErrorKind};

/// 字节流类型别名：向客户端转发的 SSE/字节数据流。
pub type ByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, Box<dyn std::error::Error + Send + Sync>>> + Send>>;

/// 缓冲首块后的结果。
pub struct BufferedResult {
    /// 第一个 data chunk（若为流式且成功）。已合并进 `remaining_stream`，
    /// 单独保留供调用方做首块检查/调试。
    #[allow(dead_code)]
    pub first_chunk: Bytes,
    /// 包含首块 + 剩余数据的完整流（供客户端消费）。
    pub remaining_stream: ByteStream,
}

/// 流首块守卫。
pub struct StreamGuard {
    /// 是否已开始向客户端发送流数据。
    stream_started: bool,
}

impl StreamGuard {
    /// 创建新的 StreamGuard。
    pub fn new() -> Self {
        Self {
            stream_started: false,
        }
    }

    /// 缓冲响应流的第一个 chunk 以检测错误。
    ///
    /// - `stream`：若为 `false`（非流式），直接返回空首块和空流。
    /// - `response_status`：上游 HTTP 响应状态码。
    /// - `response_body`：上游响应体（字节流）。
    ///
    /// 当 `stream=true` 且上游状态非 2xx 或首块包含 error 字段时，返回 `ProxyError`。
    /// 成功时返回 `BufferedResult`，其中 `remaining_stream` 包含已读取的首块。
    pub async fn buffer_first_chunk<S, E>(
        &mut self,
        stream: bool,
        response_status: StatusCode,
        mut response_body: S,
    ) -> Result<BufferedResult, ProxyError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        if !stream {
            return Ok(BufferedResult {
                first_chunk: Bytes::new(),
                remaining_stream: Box::pin(futures::stream::empty()),
            });
        }

        // 检查 HTTP 状态码
        if !response_status.is_success() {
            // 尝试读取错误体
            let error_body = match response_body.next().await {
                Some(Ok(chunk)) => String::from_utf8_lossy(&chunk).to_string(),
                _ => format!("HTTP {}", response_status.as_u16()),
            };
            return Err(ProxyError {
                kind: ProxyErrorKind::UpstreamError(response_status.as_u16()),
                status: response_status.as_u16(),
                message: error_body,
                retryable: true,
                stream_started: false,
            });
        }

        // 读取第一个 chunk
        let first_chunk = match response_body.next().await {
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                return Err(ProxyError::new(
                    ProxyErrorKind::NetworkError,
                    format!("读取流首块失败: {}", e),
                ));
            }
            None => {
                // 空响应
                return Ok(BufferedResult {
                    first_chunk: Bytes::new(),
                    remaining_stream: Box::pin(futures::stream::empty()),
                });
            }
        };

        // 检查首块是否为 SSE 错误事件。
        // 逐行解析 `data:` 行的 JSON，仅当 JSON 顶层 type=="error" 或存在 error 字段时判定为错误。
        // 避免对包含 "error" 子串的正常内容增量误报（如 {"delta":{"content":"fix the error"}}）。
        let chunk_str = String::from_utf8_lossy(&first_chunk);
        if let Some(err_text) = detect_sse_error(&chunk_str) {
            // 提取状态码（如果有）
            if let Some(code) = extract_error_code(&err_text) {
                return Err(ProxyError {
                    kind: ProxyErrorKind::UpstreamError(code),
                    status: code,
                    message: err_text,
                    retryable: true,
                    stream_started: false,
                });
            }
            return Err(ProxyError::new(
                ProxyErrorKind::UpstreamError(502),
                format!("上游流式响应返回错误: {}", err_text),
            ));
        }

        // 首块正常，标记流已开始
        self.stream_started = true;

        // 构建包含首块的完整流
        let remaining = futures::stream::once(futures::future::ready(Ok(first_chunk.clone())))
            .chain(
                response_body.map(|r| {
                    r.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }),
            );

        Ok(BufferedResult {
            first_chunk,
            remaining_stream: Box::pin(remaining),
        })
    }

    /// 流式数据是否已开始发送给客户端。
    pub fn is_stream_started(&self) -> bool {
        self.stream_started
    }
}

/// 从错误文本中提取 HTTP 状态码。
fn extract_error_code(text: &str) -> Option<u16> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(status) = value.get("status").and_then(|v| v.as_u64()) {
            return Some(status as u16);
        }
        if let Some(code) = value.get("code").and_then(|v| v.as_str()) {
            if let Ok(n) = code.parse::<u16>() {
                return Some(n);
            }
        }
        if let Some(error) = value.get("error") {
            if let Some(status) = error.get("status").and_then(|v| v.as_u64()) {
                return Some(status as u16);
            }
            if let Some(code) = error.get("code").and_then(|v| v.as_u64()) {
                return Some(code as u16);
            }
        }
    }
    None
}

/// 解析 SSE 首块，检测是否为错误事件。
///
/// 遍历每个 `data:` 行，解析其 JSON 负载；仅当 JSON 顶层满足以下任一条件才判定为错误：
/// - `type` 字段等于 `"error"`（Anthropic 风格）
/// - 存在 `error` 字段（OpenAI 风格 `{"error": {...}}`）
///
/// 避免对 JSON 字符串值中恰好出现 "error" 子串的正常内容误报。
/// 返回判定为错误的那行 JSON 文本（供 extract_error_code 进一步提取状态码）。
fn detect_sse_error(chunk: &str) -> Option<String> {
    for line in chunk.lines() {
        let trimmed = line.trim();
        // 跳过空行、注释行、事件行（event: ...）、id 行等，只处理 data: 行
        let payload = if let Some(rest) = trimmed.strip_prefix("data:") {
            rest.trim()
        } else {
            // 首块也可能是裸 JSON（非 SSE 封装的错误体），尝试整体解析
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                trimmed
            } else {
                continue;
            }
        };
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue; // 非 JSON，跳过（可能是 keepalive 注释等）
        };
        // Anthropic 风格：{"type":"error", ...}
        let is_error_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .map(|s| s == "error")
            .unwrap_or(false);
        // OpenAI 风格：{"error": {...}}
        let has_error_field = value.get("error").is_some();
        if is_error_type || has_error_field {
            return Some(payload.to_string());
        }
    }
    None
}
