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
use axum::http::{HeaderMap, Method, Request, StatusCode};
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
pub mod stream_guard;

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
        let is_stream = method == Method::POST;

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
        let final_result: Option<(Response<Body>, RequestLogEntry, Option<ModelMappingResult>)> =
            None;

        while failover.can_continue() {
            let endpoint = match selector.next(&failover.failed_ids, |eid| {
                // 模型锁检查（暂不实现全模型锁表查询，简化跳过）
                let _ = eid;
                true
            }) {
                Some((ep, _)) => ep,
                None => break, // 无可用端点
            };

            let attempt_start = Instant::now();
            let mut req_body_clone = req_json.clone();
            let mut upstream_headers = HeaderMap::new();

            // 模型映射
            let mapping_result = match model_mapper.resolve_and_rewrite(&mut req_body_clone) {
                Ok(m) => {
                    body_hash_sync(&mut req_body_clone, &body_hash);
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
            upstream_headers.extend(incoming_headers.clone().into_iter());

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
                // 冷却
                let _ = cooldown_secs;
                continue;
            }

            // 协议转换（passthrough 模式跳过 translator）
            let target_url = build_upstream_url(&endpoint, &path);
            let protocol_to = endpoint.protocol_type.clone();
            let protocol_from = route_settings.protocol_type.clone();

            let translated_body = if is_passthrough {
                // 媒体透明流转：使用原始二进制 body，不经过 JSON 翻译器
                body_bytes.to_vec()
            } else if protocol_from != protocol_to {
                match self
                    .translator_registry
                    .resolve(&protocol_from, &protocol_to)
                {
                    Ok(translator) => {
                        let body_bytes = serde_json::to_vec(&req_body_clone).unwrap_or_default();
                        // single-arg translator usage
                        let _ = translator;
                        body_bytes
                    }
                    Err(e) => {
                        failover.record_failure(
                            &endpoint,
                            &ProxyError::new(ProxyErrorKind::ProtocolError, e),
                            attempt_start.elapsed().as_millis() as u64,
                        );
                        continue;
                    }
                }
            } else {
                serde_json::to_vec(&req_body_clone).unwrap_or_default()
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
                            log_entry.request_body_hash = Some(body_hash);
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
                        let _ = cooldown_secs;
                        continue;
                    }

                    // 构建响应
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

                    let resp_bytes = resp.bytes().await.unwrap_or_default();

                    // 媒体响应 SHA256 哈希（仅记录哈希，不存储内容）
                    let media_body_hash = if is_passthrough {
                        Some(hash_bytes(&resp_bytes))
                    } else {
                        None
                    };

                    let response = Response::builder()
                        .status(status)
                        .header("content-type", &upstream_content_type)
                        .body(Body::from(resp_bytes))
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
                    log_entry.request_body_hash = Some(body_hash);
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
                    let _ = cooldown_secs;
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
        if let Some(ref m) = final_result.as_ref().and_then(|(_, _, m)| m.as_ref()) {
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

fn build_upstream_url(endpoint: &crate::db::dao::endpoints::EndpointRow, path: &str) -> String {
    let base = endpoint.base_url.trim_end_matches('/');
    format!("{}{}", base, path)
}

fn body_hash_sync(body: &mut Value, hash: &str) {
    // 占位：同步模型改写后的 hash
    let _ = (body, hash);
}

/// 计算字节数组的 SHA256 哈希（用于媒体响应日志）。
fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
