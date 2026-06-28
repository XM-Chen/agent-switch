/// GET /v1/models 聚合 handler。
///
/// 从 endpoint_models 表查询所有 enabled 端点的可用模型，
/// 去重后返回 OpenAI Models API 标准格式。
/// 支持 `?capability=` 过滤参数。
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use serde::Serialize;

use crate::app_state::AppState;
use crate::db::dao::endpoint_models;

/// OpenAI Models API 响应格式。
#[derive(Serialize)]
pub struct ModelsListResponse {
    pub object: String,
    pub data: Vec<ModelEntry>,
}

/// 单个模型条目。
#[derive(Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

/// GET /v1/models?capability=chat,images
///
/// 查询逻辑：
/// 1. 从 endpoint_models 加载所有 is_available=1 的模型。
/// 2. 若指定 ?capability= 参数，按逗号拆分多值，要求 capabilities 包含所有指定值。
/// 3. 按 model_name 去重（同模型名取第一个 matched 的 endpoint_id 作为 owned_by）。
/// 4. 组装 OpenAI Models API 标准响应。
pub async fn get_models(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Result<Json<ModelsListResponse>, (StatusCode, String)> {
    let capability_filter: Vec<String> = params
        .get("capability")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // 加载所有可用模型
    let all_models = endpoint_models::list(
        &state.db, None, // endpoint_id
        None, // source
        None, // capability（不在此层过滤，由 handler 统一处理多值）
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // 过滤 + 去重
    let mut seen: HashSet<String> = HashSet::new();
    let mut data: Vec<ModelEntry> = Vec::new();

    // 如果指定了 capability，预收集每端点的能力模型集合（用于性能）
    // 这里采用全扫描后按行过滤的方式
    'outer: for m in &all_models {
        // 跳过未可用模型
        if !m.is_available {
            continue;
        }
        // 跳过已见过的模型名
        if seen.contains(&m.model_name) {
            continue;
        }

        // 能力过滤
        if !capability_filter.is_empty() {
            let caps_str = m.capabilities.as_deref().unwrap_or("");
            for required in &capability_filter {
                if !caps_str.contains(required.as_str()) {
                    continue 'outer;
                }
            }
        }

        seen.insert(m.model_name.clone());

        // 尝试从 created_at 解析 ISO8601 → epoch
        let created = parse_iso8601_to_epoch(&m.created_at).unwrap_or(now_epoch);

        data.push(ModelEntry {
            id: m.model_name.clone(),
            object: "model".to_string(),
            created,
            owned_by: m.endpoint_id.clone(),
        });
    }

    // 按 model_name 排序
    data.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Json(ModelsListResponse {
        object: "list".to_string(),
        data,
    }))
}

/// 解析 ISO8601 时间字符串为 Unix 时间戳（秒）。
fn parse_iso8601_to_epoch(iso_str: &str) -> Option<i64> {
    use time::format_description::well_known::Iso8601;
    use time::OffsetDateTime;
    OffsetDateTime::parse(iso_str, &Iso8601::DEFAULT)
        .ok()
        .map(|dt| dt.unix_timestamp())
}
