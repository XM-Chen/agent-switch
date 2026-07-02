/// 代理转发主模块。
///
/// `RouteProxy` 是路由转发的统一入口，编排整个管道：
/// selector → model_mapper → auth_injector → translator → forwarder → logger
/// 外层由 failover 状态机驱动。
///
/// 参考 design.md §1 数据流图。
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::Response;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db::dao::route_settings::RouteSettingsRow;
use crate::http::proxy::auth_injector::inject_auth;
use crate::http::proxy::capability::{capability_to_protocol, path_to_capability};
use crate::http::proxy::error::{ProxyError, ProxyErrorKind};
use crate::http::proxy::failover::FailoverState;
use crate::http::proxy::logger::{write_log, RequestLogEntry};
use crate::http::proxy::model_mapper::{ModelMapper, ModelMappingResult};
use crate::http::proxy::selector::EndpointSelector;
use crate::services::translator::helpers;
use crate::services::translator::TranslatorRegistry;

pub mod auth_injector;
pub mod capability;
pub mod constants;
pub mod error;
pub mod failover;
pub mod logger;
pub mod model_mapper;
pub mod oauth_refresh;
pub mod selector;
pub mod sse;
pub mod stream_guard;
pub mod translate;

#[cfg(test)]
mod integration_tests;

/// 代理转发编排器。
pub struct RouteProxy {
    pub db: Arc<Mutex<Connection>>,
    pub translator_registry: Arc<TranslatorRegistry>,
    pub crypto: Option<Arc<crate::services::crypto::CryptoService>>,
    pub codex_oauth: Arc<crate::services::codex_oauth::CodexOAuthService>,
    pub data_dir: std::path::PathBuf,
    pub http_client: reqwest::Client,
}

impl RouteProxy {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        translator_registry: Arc<TranslatorRegistry>,
        crypto: Option<Arc<crate::services::crypto::CryptoService>>,
        codex_oauth: Arc<crate::services::codex_oauth::CodexOAuthService>,
        data_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            db,
            translator_registry,
            crypto,
            codex_oauth,
            data_dir,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("创建 HTTP 客户端失败"),
        }
    }

    /// 路由请求主入口。
    ///
    /// 驱动故障转移循环，依次执行：selector → model_mapper → auth → translator → forward。
    /// 对于 v1 路由，会根据子路径解析 required_capability 并传递给各管道组件。
    ///
    /// - `test_only`：测试模式，不禁用冷却中的端点，不写故障转移状态，日志标记 tool=test。
    pub async fn proxy_request(
        &self,
        route_id: &str,
        req: Request<Body>,
        test_only: bool,
    ) -> Result<Response<Body>, (StatusCode, String)> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let start = Instant::now();
        let path = req.uri().path().to_string();
        let method = req.method().clone();

        // 拆解请求
        let (parts, body) = req.into_parts();
        let body_bytes = axum::body::to_bytes(body, 1024 * 1024 * 10)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("读取请求体失败: {}", e)))?;

        let body_hash = RequestLogEntry::hash_body(&body_bytes);
        let req_json: Value = if !body_bytes.is_empty() {
            serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        // 流式判定以请求体 stream 字段为准，而非 HTTP 方法（method==POST 是错误启发式）。
        let is_stream = helpers::is_streaming(&req_json);
        let original_model = req_json
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // 加载路由设置
        let route_settings = load_route_settings(&self.db, route_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

        // v1 路由：从子路径解析 required_capability
        let required_capability = if route_id == "v1" {
            path_to_capability(&path)
        } else {
            None
        };

        // 判断是否媒体透明流转模式（images/audio 不经过 translator）
        let is_passthrough = matches!(required_capability, Some("images") | Some("audio"));

        // v1 路由：根据 capability 解析实际协议类型
        let actual_protocol_type =
            if route_settings.protocol_type == constants::PROTOCOL_OPENAI_COMPATIBLE {
                match required_capability {
                    Some(cap) => capability_to_protocol(cap)
                        .unwrap_or(&route_settings.protocol_type)
                        .to_string(),
                    None => {
                        // /v1/models 不走代理管道（在 router 层已拦截），
                        // 未识别子路径默认回退为 openai-chat
                        constants::PROTOCOL_OPENAI_CHAT.to_string()
                    }
                }
            } else {
                route_settings.protocol_type.clone()
            };

        // 初始化 selector
        let mut selector = EndpointSelector::new(&actual_protocol_type);
        selector.set_strategy(&route_settings.strategy);
        if test_only {
            selector.set_skip_cooldown(true);
        }
        selector
            .load_candidates(&self.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

        // 能力预筛（v1 路由）
        if let Some(cap) = required_capability {
            selector.set_required_capability(cap);
            selector
                .filter_by_capability(&self.db)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

            if selector.candidates().is_empty() {
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!("无可用端点具备 {} 能力", cap),
                ));
            }
        }

        let mut model_mapper = ModelMapper::new(self.db.clone(), route_id);
        if let Some(cap) = required_capability {
            model_mapper.set_required_capability(cap);
        }

        // 故障转移状态机
        let mut failover = FailoverState::new(
            route_settings.failover_enabled,
            route_settings.max_switches as u32,
            route_settings.same_account_retries as u32,
            test_only,
        );

        // 暂存 headers 用于复用
        let incoming_headers = parts.headers.clone();

        // 故障转移主循环
        // 跟踪最后一次成功的模型映射结果，供"全部失败"日志使用（替代已删除的 final_result 死变量）。
        let mut last_mapping: Option<ModelMappingResult> = None;

        while failover.can_continue() {
            let endpoint = match selector.next(&failover.failed_ids, |eid| {
                // 模型级锁检查：锁键 = endpoint_id + 原始请求 model 名。
                // 有未过期锁 → 跳过该端点（该端点此前对该模型返回 404/model_not_found）。
                match original_model.as_ref() {
                    Some(model) => {
                        match crate::db::dao::model_locks::get_active_lock(&self.db, eid, model) {
                            Ok(Some(_)) => false,
                            Ok(None) => true,
                            Err(e) => {
                                tracing::warn!("查询模型锁失败 ({}:{}): {}", eid, model, e);
                                true // 查询失败不阻塞，保守放行
                            }
                        }
                    }
                    None => true, // 无 model 字段的请求不查锁
                }
            }) {
                Some((ep, _)) => ep,
                None => break, // 无可用端点
            };

            let attempt_start = Instant::now();
            let mut req_body_clone = req_json.clone();
            let mut upstream_headers = HeaderMap::new();
            // 模型改写后重算 body hash（仅日志用途）。model_mapper 失败则 continue，
            // 不会到达使用点，故用确定赋值而非预初始化（避免 unused_assignments）。
            let loop_body_hash: String;

            // 模型映射
            let mapping_result = match model_mapper.resolve_and_rewrite(&mut req_body_clone) {
                Ok(m) => {
                    last_mapping = Some(m.clone());
                    loop_body_hash = RequestLogEntry::hash_body(
                        &serde_json::to_vec(&req_body_clone).unwrap_or_default(),
                    );
                    Some(m)
                }
                Err(e) => {
                    failover.record_failure(
                        &endpoint,
                        &ProxyError::new(ProxyErrorKind::LocalError, e),
                        attempt_start.elapsed().as_millis() as u64,
                    );
                    continue;
                }
            };

            // 复制入站 headers
            upstream_headers.extend(incoming_headers.clone());

            // 认证注入
            if let Err(e) = inject_auth(
                &endpoint,
                self.crypto.as_deref(),
                &self.codex_oauth,
                &self.data_dir,
                &self.db,
                &mut upstream_headers,
            )
            .await
            {
                let cooldown_secs = failover.record_failure(
                    &endpoint,
                    &e,
                    attempt_start.elapsed().as_millis() as u64,
                );
                persist_cooldown(&self.db, &endpoint.id, cooldown_secs, &e);
                continue;
            }

            // 协议转换（passthrough 模式跳过 translator）
            let protocol_to = endpoint.protocol_type.clone();
            // 入站协议用「已解析的实际协议」actual_protocol_type，而非路由表原始值：
            // v1 路由表里是抽象字面量 "openai-compatible"，需先按子路径解析成
            // 具体的 openai-chat / openai-responses，才能与 endpoint.protocol_type 正确比较。
            // 否则会误判为跨协议、去 resolve("openai-compatible", …) 拿不到翻译器而 502。
            let protocol_from = actual_protocol_type.clone();
            // 跨协议转发时重写 URL path 为目标协议的规范路径
            // （Claude Code 发 /v1/messages，路由到 OpenAI 端点应改为 /v1/chat/completions）。
            // 同协议 / 媒体透传保持入站原 path。
            let target_url = build_upstream_url(
                &endpoint,
                &path,
                &protocol_from,
                &protocol_to,
                is_passthrough,
            );

            let translated_body = if is_passthrough {
                // 媒体透明流转：使用原始二进制 body，不经过 JSON 翻译器
                body_bytes.to_vec()
            } else {
                let upstream_model = mapping_result
                    .as_ref()
                    .map(|m| m.upstream_model.as_str())
                    .unwrap_or("");
                match translate::build_translated_request_body(
                    &self.translator_registry,
                    &protocol_from,
                    &protocol_to,
                    &req_body_clone,
                    upstream_model,
                ) {
                    Ok(b) => b,
                    Err(e) => {
                        let elapsed = attempt_start.elapsed().as_millis() as u64;
                        let cooldown_secs = failover.record_failure(&endpoint, &e, elapsed);
                        if !test_only {
                            persist_cooldown(&self.db, &endpoint.id, cooldown_secs, &e);
                        }
                        continue;
                    }
                }
            };

            // 转发请求到上游
            let upstream_req = self
                .http_client
                .request(method.clone(), &target_url)
                .headers(upstream_headers)
                .body(translated_body)
                .send()
                .await;

            match upstream_req {
                Ok(resp) => {
                    let status = resp.status();
                    let is_success = status.is_success();

                    if !is_success {
                        let status_code = status.as_u16();
                        let err_content_type = resp
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "application/json".to_string());

                        // 缓冲错误响应体一次，供探测 / 回传客户端复用
                        let err_bytes = resp.bytes().await.unwrap_or_default();

                        let err = ProxyError::new(
                            ProxyErrorKind::UpstreamError(status_code),
                            format!("上游返回 {}", status_code),
                        );

                        // 404/model_not_found：写模型级锁（endpoint_id + 原始请求 model 名，长冷却 1h），
                        // 未来同模型请求在 selector 阶段跳过该端点。
                        if status_code == 404 {
                            if let Some(ref model) = original_model {
                                let now = time::OffsetDateTime::now_utc();
                                let until = now + time::Duration::seconds(3600);
                                let until_iso = until
                                    .format(&time::format_description::well_known::Iso8601::DEFAULT)
                                    .unwrap_or_default();
                                if let Err(e) = crate::db::dao::model_locks::set_lock(
                                    &self.db,
                                    &endpoint.id,
                                    model,
                                    &until_iso,
                                    Some("model_not_found"),
                                ) {
                                    tracing::warn!(
                                        "写入模型锁失败 ({}:{}): {}",
                                        endpoint.id,
                                        model,
                                        e
                                    );
                                }
                            }
                        }

                        // 错误分类：非可退避错误（400/405/406/413/414/415/422/501、
                        // 协议错误、本地错误等）不切换，直接把上游响应回传客户端。
                        // 参考 prd.md 故障转移策略与 design.md 状态机。
                        if !err.should_failover() || failover.stream_started {
                            // 写一条日志后直接返回上游错误响应
                            let mut log_entry =
                                RequestLogEntry::new(&request_id, Some(route_id), &path);
                            log_entry.status = Some(status_code as i64);
                            log_entry.target_endpoint_id = Some(endpoint.id.clone());
                            log_entry.upstream_endpoint = Some(target_url.clone());
                            log_entry.protocol_from = Some(protocol_from.clone());
                            log_entry.protocol_to = Some(protocol_to.clone());
                            log_entry.stream = is_stream;
                            log_entry.duration_ms = Some(start.elapsed().as_millis() as i64);
                            log_entry.request_body_hash = Some(loop_body_hash.clone());
                            log_entry.error_kind = Some(format!("{}", err));
                            log_entry.fallback_chain = Some(failover.chain_to_json());
                            if test_only {
                                log_entry.is_test = true;
                            }
                            if let Some(ref m) = mapping_result {
                                log_entry.requested_model = Some(m.original_model.clone());
                                log_entry.upstream_model = Some(m.upstream_model.clone());
                                log_entry.resolved_alias = Some(m.resolved_alias.clone());
                                log_entry.resolved_scope = Some(m.resolved_scope.clone());
                            } else {
                                log_entry.requested_model = original_model.clone();
                            }
                            let _ = write_log(&self.db, log_entry);

                            let response = Response::builder()
                                .status(status)
                                .header("content-type", &err_content_type)
                                .body(Body::from(err_bytes))
                                .unwrap();
                            return Ok(response);
                        }

                        // 可退避错误：记录失败 + 冷却，继续下一个候选
                        let cooldown_secs = failover.record_failure(
                            &endpoint,
                            &err,
                            attempt_start.elapsed().as_millis() as u64,
                        );
                        persist_cooldown(&self.db, &endpoint.id, cooldown_secs, &err);
                        continue;
                    }

                    // 成功响应：按 is_stream 分流式 / 非流式路径
                    let upstream_content_type = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "application/json".to_string());
                    let content_length = resp
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<i64>().ok());

                    // ── 流式路径：bytes_stream + StreamGuard 首块探测 ──
                    if is_stream && !is_passthrough {
                        let upstream_stream = Box::pin(resp.bytes_stream());
                        let mut guard = stream_guard::StreamGuard::new();
                        match guard
                            .buffer_first_chunk(is_stream, status, upstream_stream)
                            .await
                        {
                            Err(e) => {
                                // 首块错误：未向客户端发送任何 SSE 数据，按错误分类决定 fallback。
                                if e.should_failover() && !failover.stream_started {
                                    let cooldown_secs = failover.record_failure(
                                        &endpoint,
                                        &e,
                                        attempt_start.elapsed().as_millis() as u64,
                                    );
                                    persist_cooldown(&self.db, &endpoint.id, cooldown_secs, &e);
                                    continue;
                                }
                                // 不可切换：回传错误响应（未发 SSE header，客户端收到普通错误）
                                let err_kind = format!("{}", e);
                                let mut log_entry =
                                    RequestLogEntry::new(&request_id, Some(route_id), &path);
                                log_entry.status = Some(e.status as i64);
                                log_entry.target_endpoint_id = Some(endpoint.id.clone());
                                log_entry.upstream_endpoint = Some(target_url.clone());
                                log_entry.protocol_from = Some(protocol_from.clone());
                                log_entry.protocol_to = Some(protocol_to.clone());
                                log_entry.stream = true;
                                log_entry.duration_ms = Some(start.elapsed().as_millis() as i64);
                                log_entry.request_body_hash = Some(loop_body_hash.clone());
                                log_entry.error_kind = Some(err_kind);
                                log_entry.fallback_chain = Some(failover.chain_to_json());
                                if test_only {
                                    log_entry.is_test = true;
                                }
                                if let Some(ref m) = mapping_result {
                                    log_entry.requested_model = Some(m.original_model.clone());
                                    log_entry.upstream_model = Some(m.upstream_model.clone());
                                    log_entry.resolved_alias = Some(m.resolved_alias.clone());
                                    log_entry.resolved_scope = Some(m.resolved_scope.clone());
                                } else {
                                    log_entry.requested_model = original_model.clone();
                                }
                                let _ = write_log(&self.db, log_entry);

                                let body = serde_json::to_vec(&serde_json::json!({
                                    "error": {"type": "upstream_error", "message": format!("{}", e)}
                                }))
                                .unwrap_or_default();
                                let response = Response::builder()
                                    .status(e.status)
                                    .header("content-type", "application/json")
                                    .body(Body::from(body))
                                    .unwrap();
                                return Ok(response);
                            }
                            Ok(buffered) => {
                                // 首块正常：置 stream_started，此后禁止 fallback。
                                failover.stream_started = guard.is_stream_started();
                                let first_token_ms = start.elapsed().as_millis() as i64;

                                let out_stream: sse::ByteStream = if protocol_from != protocol_to {
                                    // 跨协议流式：逐行 translate_stream_line。
                                    // resolve 失败时退回 Passthrough（同协议直转），避免客户端挂起。
                                    let translator = self
                                        .translator_registry
                                        .resolve(&protocol_to, &protocol_from)
                                        .unwrap_or_else(|_| {
                                            self.translator_registry
                                                .resolve(&protocol_from, &protocol_from)
                                                .unwrap()
                                        });
                                    let model = mapping_result
                                        .as_ref()
                                        .map(|m| m.upstream_model.clone())
                                        .unwrap_or_default();
                                    sse::translate_stream(
                                        buffered.remaining_stream,
                                        translator,
                                        model,
                                        &protocol_from,
                                    )
                                } else {
                                    // 同协议流式：直通
                                    buffered.remaining_stream
                                };

                                let response = Response::builder()
                                    .status(status)
                                    .header("content-type", "text/event-stream")
                                    .header("cache-control", "no-cache")
                                    .header("connection", "keep-alive")
                                    .body(Body::from_stream(out_stream))
                                    .unwrap();

                                failover.record_success(
                                    &endpoint,
                                    attempt_start.elapsed().as_millis() as u64,
                                );

                                let mut log_entry =
                                    RequestLogEntry::new(&request_id, Some(route_id), &path);
                                log_entry.status = Some(status.as_u16() as i64);
                                log_entry.target_endpoint_id = Some(endpoint.id.clone());
                                log_entry.upstream_endpoint = Some(target_url.clone());
                                log_entry.protocol_from = Some(protocol_from.clone());
                                log_entry.protocol_to = Some(protocol_to.clone());
                                log_entry.stream = true;
                                log_entry.duration_ms = Some(start.elapsed().as_millis() as i64);
                                log_entry.first_token_ms = Some(first_token_ms);
                                log_entry.request_body_hash = Some(loop_body_hash.clone());
                                log_entry.fallback_chain = Some(failover.chain_to_json());
                                if test_only {
                                    log_entry.is_test = true;
                                }
                                if let Some(ref m) = mapping_result {
                                    log_entry.requested_model = Some(m.original_model.clone());
                                    log_entry.upstream_model = Some(m.upstream_model.clone());
                                    log_entry.resolved_alias = Some(m.resolved_alias.clone());
                                    log_entry.resolved_scope = Some(m.resolved_scope.clone());
                                } else {
                                    log_entry.requested_model = original_model.clone();
                                }
                                let _ = write_log(&self.db, log_entry);

                                return Ok(response);
                            }
                        }
                    }

                    // ── 非流式路径（含 passthrough 媒体透明流转）──
                    let resp_bytes = resp.bytes().await.unwrap_or_default();

                    // 跨协议响应转换（方向反转：to → from，将上游协议响应转回入站协议）
                    let out_bytes = if !is_passthrough && protocol_from != protocol_to {
                        translate::translate_response_body(
                            &self.translator_registry,
                            &protocol_to,
                            &protocol_from,
                            &resp_bytes,
                        )
                    } else {
                        resp_bytes.to_vec()
                    };

                    // 媒体响应 SHA256 哈希（仅记录哈希，不存储内容）
                    let media_body_hash = if is_passthrough {
                        Some(hash_bytes(&out_bytes))
                    } else {
                        None
                    };

                    let response = Response::builder()
                        .status(status)
                        .header("content-type", &upstream_content_type)
                        .body(Body::from(out_bytes))
                        .unwrap();

                    failover.record_success(&endpoint, attempt_start.elapsed().as_millis() as u64);

                    // 构建日志
                    let mut log_entry = RequestLogEntry::new(&request_id, Some(route_id), &path);
                    log_entry.status = Some(status.as_u16() as i64);
                    log_entry.target_endpoint_id = Some(endpoint.id.clone());
                    log_entry.upstream_endpoint = Some(target_url);
                    log_entry.protocol_from = Some(protocol_from);
                    log_entry.protocol_to = Some(protocol_to);
                    log_entry.stream = is_stream;
                    log_entry.duration_ms = Some(start.elapsed().as_millis() as i64);
                    log_entry.request_body_hash = Some(loop_body_hash);
                    log_entry.fallback_chain = Some(failover.chain_to_json());
                    if test_only {
                        log_entry.is_test = true;
                    }
                    if let Some(ref m) = mapping_result {
                        log_entry.requested_model = Some(m.original_model.clone());
                        log_entry.upstream_model = Some(m.upstream_model.clone());
                        log_entry.resolved_alias = Some(m.resolved_alias.clone());
                        log_entry.resolved_scope = Some(m.resolved_scope.clone());
                    } else {
                        log_entry.requested_model = original_model.clone();
                    }

                    // 媒体日志字段
                    if is_passthrough {
                        log_entry.media_type = Some(upstream_content_type);
                        log_entry.content_length = content_length;
                        log_entry.body_sha256_hash = media_body_hash;
                    }

                    let _ = write_log(&self.db, log_entry);

                    return Ok(response);
                }
                Err(e) => {
                    let err = ProxyError::new(
                        if e.is_timeout() {
                            ProxyErrorKind::Timeout
                        } else if e.is_connect() {
                            ProxyErrorKind::NetworkError
                        } else {
                            ProxyErrorKind::UpstreamError(502)
                        },
                        format!("上游请求失败: {}", e),
                    );
                    let cooldown_secs = failover.record_failure(
                        &endpoint,
                        &err,
                        attempt_start.elapsed().as_millis() as u64,
                    );
                    persist_cooldown(&self.db, &endpoint.id, cooldown_secs, &err);
                    continue;
                }
            }
        }

        // 全部失败
        let last_error = failover
            .last_error
            .as_ref()
            .map(|e| format!("{}", e))
            .unwrap_or_else(|| "所有候选端点均已失败".to_string());

        // 写错误日志
        let mut log_entry = RequestLogEntry::new(&request_id, Some(route_id), &path);
        log_entry.status = Some(
            failover
                .last_error
                .as_ref()
                .map(|e| e.status as i64)
                .unwrap_or(502),
        );
        log_entry.error_kind = Some(last_error.clone());
        log_entry.fallback_chain = Some(failover.chain_to_json());
        log_entry.request_body_hash = Some(body_hash);
        log_entry.duration_ms = Some(start.elapsed().as_millis() as i64);
        if test_only {
            log_entry.is_test = true;
        }
        if let Some(ref m) = last_mapping {
            log_entry.requested_model = Some(m.original_model.clone());
        } else {
            log_entry.requested_model = original_model;
        }
        let _ = write_log(&self.db, log_entry);

        Err((StatusCode::BAD_GATEWAY, last_error))
    }
}

async fn load_route_settings(
    db: &Mutex<Connection>,
    route_id: &str,
) -> Result<RouteSettingsRow, String> {
    use crate::db::dao::route_settings;
    route_settings::get(db, route_id)?.ok_or_else(|| format!("路由 '{}' 未配置", route_id))
}

/// 构建上游转发 URL。
///
/// 跨协议转发时（`protocol_from != protocol_to` 且非媒体透传），入站路径按目标协议
/// 重写为规范路径：anthropic→`/v1/messages`、openai-chat→`/v1/chat/completions`、
/// openai-responses→`/v1/responses`。同协议或媒体透传保持入站原 path。
fn build_upstream_url(
    endpoint: &crate::db::dao::endpoints::EndpointRow,
    path: &str,
    protocol_from: &str,
    protocol_to: &str,
    is_passthrough: bool,
) -> String {
    let base = endpoint.base_url.trim_end_matches('/');
    let effective_path = if !is_passthrough && protocol_from != protocol_to {
        protocol_canonical_path(protocol_to).unwrap_or(path)
    } else {
        path
    };
    format!("{}{}", base, effective_path)
}

/// 协议 → 规范请求路径。未知协议返回 None（保持入站原 path）。
fn protocol_canonical_path(protocol: &str) -> Option<&'static str> {
    match protocol {
        constants::PROTOCOL_ANTHROPIC => Some("/v1/messages"),
        constants::PROTOCOL_OPENAI_CHAT => Some("/v1/chat/completions"),
        constants::PROTOCOL_OPENAI_RESPONSES => Some("/v1/responses"),
        _ => None,
    }
}

/// 将端点冷却时长持久化到 DB（写 `cooldown_until` / `last_failure_at` / `last_error_kind`）。
///
/// `cooldown_secs <= 0`（含 test_only 模式 record_failure 返回 0）时直接返回，不写。
/// 写失败只记日志，不中断故障转移主流程。
fn persist_cooldown(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    cooldown_secs: i64,
    err: &ProxyError,
) {
    use crate::db::dao::endpoints::{self, EndpointUpdate};
    use time::format_description::well_known::Iso8601;

    if cooldown_secs <= 0 {
        return;
    }

    let now = time::OffsetDateTime::now_utc();
    let until = now + time::Duration::seconds(cooldown_secs);
    let fmt = |dt: time::OffsetDateTime| dt.format(&Iso8601::DEFAULT).unwrap_or_default();

    if let Err(e) = endpoints::update(
        db,
        endpoint_id,
        EndpointUpdate {
            cooldown_until: Some(Some(fmt(until))),
            last_failure_at: Some(Some(fmt(now))),
            last_error_kind: Some(Some(format!("{}", err.kind))),
            ..Default::default()
        },
    ) {
        tracing::warn!("写入端点 '{}' 冷却失败: {}", endpoint_id, e);
    }
}

/// 计算字节数组的 SHA256 哈希（用于媒体响应日志）。
fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
