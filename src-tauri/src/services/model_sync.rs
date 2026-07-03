use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

use crate::app_state::AppState;
use crate::db::dao::app_metadata;
use crate::db::dao::endpoint_models;
use crate::db::dao::endpoints::{self, EndpointRow};
use crate::db::dao::request_logs;

const SETTING_AUTO_REFRESH: &str = "auto_model_refresh_enabled";
const SETTING_LAST_SYNC_AT: &str = "last_model_sync_at";
const SETTING_LAST_SYNC_ERROR: &str = "last_model_sync_error";

/// 日志保留上限。超过此条数的最旧日志会在每次模型同步时被清理。
const MAX_LOG_ROWS: i64 = 5000;

/// 模型刷新服务。
pub struct ModelSyncService {
    pub is_running: Arc<Mutex<bool>>,
}

impl ModelSyncService {
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(Mutex::new(false)),
        }
    }

    /// 是否启用自动刷新。
    pub fn is_auto_refresh_enabled(db: &Mutex<Connection>) -> bool {
        app_metadata::get(db, SETTING_AUTO_REFRESH)
            .ok()
            .flatten()
            .as_deref()
            == Some("true")
    }

    pub fn set_auto_refresh(db: &Mutex<Connection>, enabled: bool) -> Result<(), String> {
        app_metadata::set(
            db,
            SETTING_AUTO_REFRESH,
            if enabled { "true" } else { "false" },
        )
    }

    /// 手动触发全量刷新。返回刷新的端点数和错误。
    pub async fn sync_all(&self, app_state: Arc<AppState>) -> Result<SyncReport, String> {
        // 互斥：避免手动 + 定时同时运行。
        {
            let mut guard = self
                .is_running
                .lock()
                .map_err(|e| format!("锁失败: {}", e))?;
            if *guard {
                return Err("已有刷新任务在运行".to_string());
            }
            *guard = true;
        }

        let result = self.do_sync_all(app_state).await;

        {
            let mut guard = self
                .is_running
                .lock()
                .map_err(|e| format!("锁失败: {}", e))?;
            *guard = false;
        }
        result
    }

    async fn do_sync_all(&self, app_state: Arc<AppState>) -> Result<SyncReport, String> {
        let endpoints_list = endpoints::list_enabled(&app_state.db)?;
        let sync_time = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| format!("时间格式化失败: {}", e))?;

        let mut report = SyncReport {
            synced_at: sync_time.clone(),
            ..Default::default()
        };

        // 第一版顺序刷新每个端点。后续可改为 tokio::spawn + Semaphore 按主机分组限流。

        for ep in endpoints_list {
            match self.sync_one_endpoint(&app_state, &ep, &sync_time).await {
                Ok(count) => {
                    report.succeeded.push(SyncResultItem {
                        endpoint_id: ep.id.clone(),
                        endpoint_name: ep.name.clone(),
                        model_count: count,
                    });
                }
                Err(e) => {
                    tracing::warn!("端点 {} 刷新失败: {}", ep.name, e);
                    report.failed.push(SyncResultItem {
                        endpoint_id: ep.id.clone(),
                        endpoint_name: ep.name.clone(),
                        model_count: 0,
                    });
                    report.errors.push(format!("{}: {}", ep.name, e));
                }
            }
        }

        // 更新刷新时间戳。
        let _ = app_metadata::set(&app_state.db, SETTING_LAST_SYNC_AT, &sync_time);
        if report.errors.is_empty() {
            let _ = app_metadata::set(&app_state.db, SETTING_LAST_SYNC_ERROR, "");
        } else {
            let _ = app_metadata::set(
                &app_state.db,
                SETTING_LAST_SYNC_ERROR,
                &report.errors.join("; "),
            );
        }

        // 顺带清理过期日志，防止 request_logs 无限增长。
        // 容错：清理失败不影响同步结果。
        if let Err(e) = request_logs::prune_old(&app_state.db, MAX_LOG_ROWS) {
            tracing::warn!("清理旧请求日志失败: {}", e);
        }

        Ok(report)
    }

    /// 刷新单个端点的模型列表。
    ///
    /// 所有 upsert + mark_unavailable 在同一个事务中完成，
    /// 避免网络中断导致部分模型被误标为不可用。
    async fn sync_one_endpoint(
        &self,
        app_state: &Arc<AppState>,
        ep: &EndpointRow,
        sync_time: &str,
    ) -> Result<usize, String> {
        let models = fetch_models_from_endpoint(app_state, ep).await?;

        if models.is_empty() {
            tracing::warn!(
                "端点 '{}' 的 /v1/models 返回空 data 数组，跳过本次可用性标记以避免误下线已有模型",
                ep.name
            );
            return Ok(0);
        }

        let endpoint_id = ep.id.clone();
        let sync_time_owned = sync_time.to_string();

        // 在单个事务中执行所有 DB 写入
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| format!("时间格式化失败: {}", e))?;

        let conn = app_state
            .db
            .lock()
            .map_err(|e| format!("无法锁定数据库: {}", e))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("开始事务失败: {}", e))?;

        let mut synced_count = 0usize;
        for m in &models {
            let id = uuid::Uuid::new_v4().to_string();
            let new_model = endpoint_models::NewEndpointModel {
                id,
                endpoint_id: endpoint_id.clone(),
                model_name: m.id.clone(),
                display_name: m.id.clone(),
                source: "synced".to_string(),
                capabilities: m.capabilities.clone(),
                context_window: m.context_window,
                last_seen_at: Some(sync_time_owned.clone()),
            };
            if endpoint_models::upsert_synced_in_tx(&tx, &new_model, &now)? {
                synced_count += 1;
            } else {
                tracing::warn!(
                    "端点 '{}' 的同步模型 '{}' 与 custom 模型重名，已保留 custom 行并跳过同步覆盖",
                    ep.name,
                    m.id
                );
            }
        }

        // 标记本次未返回的 synced 模型为不可用
        endpoint_models::mark_unavailable_except_in_tx(&tx, &endpoint_id, &sync_time_owned, &now)?;

        tx.commit()
            .map_err(|e| format!("提交事务失败: {}", e))?;

        Ok(synced_count)
    }
}

/// 从上游获取模型列表。
pub async fn fetch_models_from_endpoint(
    app_state: &Arc<AppState>,
    ep: &EndpointRow,
) -> Result<Vec<FetchedModel>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/models", ep.base_url.trim_end_matches('/'));

    let mut req = client.get(&url);
    // 注入凭据（如果有）。
    if let Some(crypto) = app_state.crypto.as_ref() {
        if let Some(blob) = &ep.api_key_encrypted {
            let plain = crypto
                .decrypt(blob, ep.id.as_bytes())
                .map_err(|e| format!("解密 API Key 失败: {}", e))?;
            let v: serde_json::Value =
                serde_json::from_slice(&plain).map_err(|e| format!("解析凭据失败: {}", e))?;
            if let Some(key) = v.get("api_key").and_then(|k| k.as_str()) {
                req = req.bearer_auth(key);
            }
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 /v1/models 失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("上游返回 {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 /v1/models 响应失败: {}", e))?;

    let data = body
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "/v1/models 响应缺少 data 数组".to_string())?;

    let mut models = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "模型缺少 id 字段".to_string())?;
        models.push(FetchedModel {
            id: id.to_string(),
            capabilities: capabilities_for_item(item, &ep.protocol_type)?,
            context_window: context_window_for_item(item),
        });
    }
    Ok(models)
}

/// 为单个 /v1/models 条目推断 capabilities。
///
/// 优先采用上游返回的 capabilities 字段（数组或逗号分隔字符串）；
/// 若上游未提供，则按端点 protocol_type 推导一个保守的基线能力：
/// - openai-responses → `["responses"]`
/// - 其它（openai-chat / anthropic / openai-compatible）→ `["chat"]`
///
/// 不再硬编码 `tool_calling` / `vision_input`：这些是模型特定能力，仅凭
/// /v1/models 列表无法可靠判断，应由用户通过自定义模型显式标注。
fn capabilities_for_item(
    item: &serde_json::Value,
    protocol_type: &str,
) -> Result<Option<String>, String> {
    if let Some(caps) = extract_upstream_capabilities(item) {
        return Ok(Some(serde_json::to_string(&caps).map_err(|e| {
            format!("序列化 capabilities 失败: {}", e)
        })?));
    }

    let baseline: &[&str] = match protocol_type {
        "openai-responses" => &["responses"],
        _ => &["chat"],
    };
    Ok(Some(
        serde_json::to_string(baseline)
            .map_err(|e| format!("序列化 capabilities 失败: {}", e))?,
    ))
}

/// 从上游 /v1/models 条目中提取 capabilities，若未声明返回 None。
///
/// 兼容两种常见格式：
/// - 数组：`"capabilities": ["chat", "streaming"]`
/// - 逗号分隔字符串：`"capabilities": "chat,streaming"`
fn extract_upstream_capabilities(item: &serde_json::Value) -> Option<Vec<String>> {
    let caps = item.get("capabilities")?;
    if let Some(arr) = caps.as_array() {
        let out: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    } else if let Some(s) = caps.as_str() {
        let out: Vec<String> = s
            .split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    } else {
        None
    }
}

/// 从上游 /v1/models 条目中提取 context_window，若未声明返回 None。
///
/// 兼容常见字段名：`context_window` / `context_length` / `max_context_length`。
fn context_window_for_item(item: &serde_json::Value) -> Option<i64> {
    item.get("context_window")
        .and_then(|v| v.as_i64())
        .or_else(|| item.get("context_length").and_then(|v| v.as_i64()))
        .or_else(|| item.get("max_context_length").and_then(|v| v.as_i64()))
}

#[derive(Debug, Default, serde::Serialize)]
pub struct SyncReport {
    pub synced_at: String,
    pub succeeded: Vec<SyncResultItem>,
    pub failed: Vec<SyncResultItem>,
    pub errors: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct SyncResultItem {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub model_count: usize,
}

#[derive(Debug)]
pub struct FetchedModel {
    pub id: String,
    pub capabilities: Option<String>,
    pub context_window: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_use_upstream_field_when_present() {
        let item = serde_json::json!({
            "id": "gpt-image-1",
            "capabilities": ["images", "vision_input"],
            "context_window": 128000,
        });

        let caps = capabilities_for_item(&item, "openai-chat")
            .expect("能力推导应成功")
            .expect("应有能力字段");
        let parsed: Vec<String> = serde_json::from_str(&caps).expect("能力应为 JSON 数组");
        assert_eq!(parsed, vec!["images", "vision_input"]);
        assert_eq!(context_window_for_item(&item), Some(128000));
    }

    #[test]
    fn capabilities_fallback_is_conservative_and_does_not_claim_tool_calling() {
        let chat_item = serde_json::json!({ "id": "gpt-4o" });
        let chat_caps = capabilities_for_item(&chat_item, "openai-chat")
            .expect("能力推导应成功")
            .expect("应有能力字段");
        let parsed: Vec<String> = serde_json::from_str(&chat_caps).expect("能力应为 JSON 数组");
        assert_eq!(parsed, vec!["chat"]);
        assert!(!parsed.iter().any(|cap| cap == "tool_calling"));

        let responses_item = serde_json::json!({ "id": "gpt-5" });
        let responses_caps = capabilities_for_item(&responses_item, "openai-responses")
            .expect("能力推导应成功")
            .expect("应有能力字段");
        let parsed: Vec<String> =
            serde_json::from_str(&responses_caps).expect("能力应为 JSON 数组");
        assert_eq!(parsed, vec!["responses"]);
    }
}
