/// 链路测试 API。
///
/// POST /api/tests — 对指定路由发起真实链路测试（test_only 模式）。
///
/// 构建完整请求体走 RouteProxy 管道，返回上游响应。
/// 测试模式不写冷却/故障转移状态，日志标记 tool=test。
///
/// 流式模式：返回上游原始 SSE 响应（`text/event-stream`）。
/// 非流式模式：返回 JSON 包装体 `{ status, body, duration_ms, endpoint_id, error }`。
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Json, State};
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;

use crate::app_state::AppState;

/// 测试请求。
#[derive(Deserialize)]
pub struct TestRequest {
    /// 路由 ID（claude-code | codex | v1）。
    pub route: String,
    /// 转发路径（如 /v1/messages、/v1/chat/completions）。
    pub path: String,
    /// 可选模型名，不提供则使用默认。
    pub model: Option<String>,
    /// 测试 prompt 文本。
    pub prompt: String,
    /// 是否流式输出。
    #[serde(default)]
    pub stream: bool,
}

/// POST /api/tests — 发起链路测试。
///
/// 构建请求体 → 调用 RouteProxy::proxy_request(route, req, test_only=true)。
///
/// - 非流式：返回 `{ status, body, duration_ms, endpoint_id, error, fallback_chain }`。
/// - 流式：透传上游 `text/event-stream` 响应体，额外头部附带元数据。
pub async fn run_test(
    State(state): State<Arc<AppState>>,
    Json(test_req): Json<TestRequest>,
) -> Result<Response<Body>, (StatusCode, String)> {
    let start = std::time::Instant::now();

    // 校验路由
    match test_req.route.as_str() {
        "claude-code" | "codex" | "v1" => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "不支持的路由: {}，仅支持 claude-code / codex / v1",
                    test_req.route
                ),
            ));
        }
    }

    // 根据路由类型构建请求体
    let body_value = build_test_body(&test_req)?;

    // 构建 Axum Request
    let body_bytes = serde_json::to_vec(&body_value)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化请求体失败: {}", e)))?;

    let req = Request::builder()
        .method(Method::POST)
        .uri(&test_req.path)
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("构建测试请求失败: {}", e)))?;

    // 获取 RouteProxy 并调用
    let route_proxy = state.route_proxy.read().await;
    let proxy = route_proxy
        .as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "代理服务未初始化".to_string()))?;

    let upstream_resp = proxy.proxy_request(&test_req.route, req, true).await?;

    let duration_ms = start.elapsed().as_millis() as u64;

    if test_req.stream {
        // 流式模式：透传上游响应，附加元数据头部
        let (mut parts, body) = upstream_resp.into_parts();
        parts.headers.insert(
            "x-test-duration-ms",
            duration_ms.to_string().parse().unwrap(),
        );
        let resp = Response::from_parts(parts, body);
        Ok(resp)
    } else {
        // 非流式模式：缓冲响应体，返回 JSON 包装
        let status = upstream_resp.status();
        let (parts, body) = upstream_resp.into_parts();
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024 * 10)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取响应体失败: {}", e),
                )
            })?;

        let resp_body: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);

        // 从响应头中提取 endpoint_id（如果有）
        let endpoint_id = parts
            .headers
            .get("x-endpoint-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let result = serde_json::json!({
            "status": status.as_u16(),
            "body": resp_body,
            "duration_ms": duration_ms,
            "endpoint_id": endpoint_id,
            "error": null
        });

        let resp = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&result).unwrap()))
            .unwrap();
        Ok(resp)
    }
}

/// 根据路由类型构建测试请求 JSON 体。
fn build_test_body(req: &TestRequest) -> Result<Value, (StatusCode, String)> {
    match req.route.as_str() {
        "claude-code" => {
            let model = req
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            Ok(serde_json::json!({
                "model": model,
                "max_tokens": 1024,
                "messages": [
                    {"role": "user", "content": req.prompt}
                ]
            }))
        }
        "codex" => {
            let model = req
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
            Ok(serde_json::json!({
                "model": model,
                "input": req.prompt
            }))
        }
        "v1" => {
            let model = req
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o".to_string());
            Ok(serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "user", "content": req.prompt}
                ],
                "stream": req.stream
            }))
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            format!("不支持的路由: {}", req.route),
        )),
    }
}
