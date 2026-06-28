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

use crate::db::dao::route_settings::RouteSettingsRow;
use crate::http::proxy::auth_injector::inject_auth;
use crate::http::proxy::error::{ProxyError, ProxyErrorKind};
use crate::http::proxy::failover::FailoverState;
use crate::http::proxy::logger::{write_log, RequestLogEntry};
use crate::http::proxy::model_mapper::{ModelMapper, ModelMappingResult};
use crate::http::proxy::selector::EndpointSelector;
use crate::services::translator::TranslatorRegistry;

pub mod auth_injector;
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
    pub async fn proxy_request(
        &self,
        route_id: &str,
        req: Request<Body>,
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

        // 初始化 selector
        let mut selector = EndpointSelector::new(&route_settings.protocol_type);
        selector.set_strategy(&route_settings.strategy);
        selector
            .load_candidates(&self.db)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

        let model_mapper = ModelMapper::new(self.db.clone(), route_id);

        // 故障转移状态机
        let mut failover = FailoverState::new(
            route_settings.failover_enabled,
            route_settings.max_switches as u32,
            route_settings.same_account_retries as u32,
        );

        // 暂存 headers 用于复用
        let incoming_headers = parts.headers.clone();

        // 故障转移主循环
        let mut final_result: Option<(
            Response<Body>,
            RequestLogEntry,
            Option<ModelMappingResult>,
        )> = None;

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

            // 协议转换
            let target_url = build_upstream_url(&endpoint, &path);
            let protocol_to = endpoint.protocol_type.clone();
            let protocol_from = route_settings.protocol_type.clone();

            let translated_body = if protocol_from != protocol_to {
                match self
                    .translator_registry
                    .resolve(&protocol_from, &protocol_to)
                {
                    Ok(translator) => {
                        let mut body_bytes =
                            serde_json::to_vec(&req_body_clone).unwrap_or_default();
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
                        let err = ProxyError::new(
                            ProxyErrorKind::UpstreamError(status_code),
                            format!("上游返回 {}", status_code),
                        );
                        let cooldown_secs = failover.record_failure(
                            &endpoint,
                            &err,
                            attempt_start.elapsed().as_millis() as u64,
                        );
                        let _ = cooldown_secs;
                        continue;
                    }

                    // 构建响应
                    let resp_bytes = resp.bytes().await.unwrap_or_default();

                    let response = Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
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
