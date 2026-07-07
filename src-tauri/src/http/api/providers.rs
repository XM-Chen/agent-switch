use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::app_state::AppState;
use crate::db::dao::providers::{self, NewProvider, ProviderRow, ProviderUpdate};
use crate::db::dao::tool_takeover as takeover_dao;
use crate::services::importers::ccs as ccs_importer;
use crate::services::tool_takeover::{self, Tool};

/// Provider 响应体。settings_config / meta 以 JSON Value 返回。
#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: String,
    pub app_type: String,
    pub name: String,
    pub mode: String,
    pub settings_config: Value,
    pub is_current: bool,
    pub category: Option<String>,
    pub sort_index: Option<i64>,
    pub notes: Option<String>,
    pub meta: Value,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ProviderRow> for ProviderResponse {
    fn from(r: ProviderRow) -> Self {
        Self {
            id: r.id,
            app_type: r.app_type,
            name: r.name,
            mode: r.mode,
            settings_config: serde_json::from_str(&r.settings_config).unwrap_or(Value::Null),
            is_current: r.is_current,
            category: r.category,
            sort_index: r.sort_index,
            notes: r.notes,
            meta: serde_json::from_str(&r.meta).unwrap_or(Value::Null),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// 列表查询参数。
#[derive(Deserialize)]
pub struct ListQuery {
    pub app_type: Option<String>,
}

/// 创建 provider 请求（不激活）。
#[derive(Deserialize)]
pub struct CreateProviderRequest {
    pub app_type: String,
    pub name: String,
    /// 缺省 "proxy"。
    pub mode: Option<String>,
    pub settings_config: Value,
    pub category: Option<String>,
    pub notes: Option<String>,
}

/// 更新 provider 请求（部分字段，不含 is_current）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
#[derive(Deserialize)]
pub struct UpdateProviderRequest {
    pub name: Option<String>,
    pub mode: Option<String>,
    pub settings_config: Option<Value>,
    pub category: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    /// Claude Code common config 三态开关（`meta.common_config_enabled`）。
    ///
    /// 嵌套 `Option`：外层 `Some` 表示「更新该开关」，内层 `Some(bool)` 为显式启用/禁用、
    /// `None` 为清除（回落默认）。仅对 claude-code provider 有意义。
    pub common_config_enabled: Option<Option<bool>>,
}

/// 切换响应：warnings 承载非致命提示。
#[derive(Serialize, Debug)]
pub struct SwitchResponse {
    pub warnings: Vec<String>,
}

/// reorder 单项。
#[derive(Deserialize)]
pub struct ReorderItem {
    pub id: String,
    pub sort_index: i64,
}

/// 批量重排请求。
#[derive(Deserialize)]
pub struct ReorderRequest {
    pub items: Vec<ReorderItem>,
}

/// ccs 导入请求：仅 items（每项含 original_id + imported_name）。
#[derive(Deserialize)]
pub struct ImportCcsRequest {
    pub items: Vec<ccs_importer::ImportItem>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        // 固定段必须先于 /{id} 注册，避免被参数路由吞掉。
        .route("/reorder", post(reorder))
        .route("/import-ccs/detect", post(detect_ccs))
        .route("/import-ccs", post(import_ccs))
        .route("/{id}", get(get_one).put(update).delete(delete))
        .route("/{id}/switch", post(switch))
}

/// GET /api/providers?app_type=<claude-code|codex> — 按 sort_index 升序列出。
async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ProviderResponse>>, (StatusCode, String)> {
    let app_type = q
        .app_type
        .ok_or((StatusCode::BAD_REQUEST, "缺少 app_type 参数".to_string()))?;
    let rows = providers::list_by_app(&state.db, &app_type)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(ProviderResponse::from).collect()))
}

/// GET /api/providers/{id} — 单个 provider。
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ProviderResponse>, (StatusCode, String)> {
    let row = providers::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match row {
        Some(r) => Ok(Json(ProviderResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "provider 不存在".to_string())),
    }
}

/// POST /api/providers — 创建 provider（is_current 恒 0，sort_index 自动追加）。
async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<ProviderResponse>), (StatusCode, String)> {
    let sort_index = providers::next_sort_index(&state.db, &req.app_type)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let new = NewProvider {
        id: uuid::Uuid::new_v4().to_string(),
        app_type: req.app_type,
        name: req.name,
        mode: req.mode.unwrap_or_else(|| "proxy".to_string()),
        settings_config: req.settings_config.to_string(),
        category: req.category,
        sort_index: Some(sort_index),
        notes: req.notes,
        meta: "{}".to_string(),
    };
    let row =
        providers::create(&state.db, new).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(ProviderResponse::from(row))))
}

/// PUT /api/providers/{id} — 更新部分字段（不含 is_current / sort_index）。
async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    // common config 三态开关写入 meta：需以现有 meta 为基底，保留其它键（如 snapshot）。
    let meta = match req.common_config_enabled {
        Some(enabled) => {
            let existing = providers::get(&state.db, &id)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                .ok_or_else(|| (StatusCode::NOT_FOUND, "provider 不存在".to_string()))?;
            let new_meta =
                tool_takeover::claude_snapshot::common_enabled_into_meta(&existing.meta, enabled)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            Some(new_meta)
        }
        None => None,
    };

    let upd = ProviderUpdate {
        name: req.name,
        mode: req.mode,
        settings_config: req.settings_config.map(|v| v.to_string()),
        category: req.category,
        sort_index: None,
        notes: req.notes,
        meta,
    };
    providers::update(&state.db, &id, upd).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/providers/{id} — 删除；若删除的是 current，清 current + 清 tool_takeover 悬挂引用。
async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Some(p) =
        providers::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        if p.is_current {
            providers::clear_current(&state.db, &p.app_type)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

            // 若对应工具的接管状态仍指向被删 provider，重置为 proxy 且清空 active_provider_id，
            // 避免 direct 模式悬挂引用一个不存在的 provider。
            if let Some(tool) = Tool::from_str(&p.app_type) {
                if let Some(st) = takeover_dao::get_state(&state.db, tool.as_str())
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                {
                    if st.active_provider_id.as_deref() == Some(id.as_str()) {
                        takeover_dao::set_mode(&state.db, tool.as_str(), "proxy", None)
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
                    }
                }
            }
        }
    }

    providers::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/providers/reorder — 批量更新 sort_index。
async fn reorder(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReorderRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let updates: Vec<(String, i64)> = req
        .items
        .into_iter()
        .map(|i| (i.id, i.sort_index))
        .collect();
    providers::update_sort_order(&state.db, &updates)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/providers/import-ccs/detect — 探测本地 ccs 安装并返回预览列表（只读）。
async fn detect_ccs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ccs_importer::DetectResponse>, (StatusCode, String)> {
    let resp = ccs_importer::detect(&state.db, None, None)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

/// POST /api/providers/import-ccs — 批量导入 ccs 渠道：建 endpoint + direct provider。
async fn import_ccs(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportCcsRequest>,
) -> Result<Json<ccs_importer::ImportResponse>, (StatusCode, String)> {
    let resp = ccs_importer::import(&state.db, state.crypto.as_deref(), None, None, req.items)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

/// POST /api/providers/{id}/switch — 切换：设 is_current + 按 mode 接管，失败回滚。
async fn switch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SwitchResponse>, (StatusCode, String)> {
    let resp = perform_switch(&state.db, &id, |provider, prev, tool| {
        match tool {
            // Claude Code：快照切换编排（回填保护 + Common Config 三层）。
            Tool::ClaudeCode => tool_takeover::switch_claude(
                &state.db,
                &state.data_dir,
                prev,
                provider,
                state.crypto.as_deref(),
            ),
            // Codex 等：沿用「仅连接层」接管（保持现状，不引入快照模型）。
            _ => match provider.mode.as_str() {
                "direct" => tool_takeover::enable_direct(
                    &state.db,
                    tool,
                    &state.data_dir,
                    provider,
                    state.crypto.as_deref(),
                ),
                _ => tool_takeover::enable(&state.db, tool, &state.data_dir),
            }
            .map(|()| Vec::new()),
        }
    })?;
    Ok(Json(resp))
}

/// 切换核心：与 AppState 解耦，便于用注入的接管闭包单测成功与回滚。
///
/// 顺序（R3）：查目标 → 解析 app_type → **先设 is_current** → 接管 →
/// 接管失败**回滚 is_current**（恢复切换前的 current，或清空），保证 DB 与工具配置一致。
fn perform_switch<F>(
    db: &Mutex<Connection>,
    id: &str,
    takeover: F,
) -> Result<SwitchResponse, (StatusCode, String)>
where
    F: FnOnce(&ProviderRow, Option<&ProviderRow>, Tool) -> Result<Vec<String>, String>,
{
    // 1. 查目标 provider。
    let provider = providers::get(db, id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "provider 不存在".to_string()))?;

    // 2. 解析 app_type → Tool，且必须支持接管。
    let tool = Tool::from_str(&provider.app_type)
        .filter(|t| t.supports_takeover())
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!("app_type '{}' 不支持接管切换", provider.app_type),
        ))?;

    // 3. 记录切换前的 current（完整行：既用于回填快照，也用于失败回滚）。
    let prev_current = providers::get_current(db, &provider.app_type)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    // 切走前 provider 若就是目标自身（重复切换同一个），回填 sink 用 target 更新态即可，
    // 无需把 target 当作独立的 prev。
    let prev_for_backfill = prev_current.as_ref().filter(|p| p.id != provider.id);

    // 4. 先设 is_current（DB partial unique index 保证互斥）。
    providers::set_current(db, id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // 5. 按 mode 接管。
    match takeover(&provider, prev_for_backfill, tool) {
        Ok(warnings) => Ok(SwitchResponse { warnings }),
        Err(e) => {
            // 6. 接管失败 → 回滚 is_current 到切换前状态。
            let rollback = match &prev_current {
                Some(prev) => providers::set_current(db, &prev.id),
                None => providers::clear_current(db, &provider.app_type),
            };
            if let Err(re) = rollback {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("接管失败且回滚失败: 接管错误={}; 回滚错误={}", e, re),
                ));
            }
            // crypto 不可用属服务不可用语义；其余归内部错误。
            let code = if e.contains("加密服务不可用") {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            Err((code, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::dao::providers::NewProvider;
    use crate::db::migrations::run_migrations;
    use rusqlite::Connection;
    use std::sync::Mutex;

    fn setup() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("无法创建内存数据库");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移应成功");
        db
    }

    fn new_provider(id: &str, app_type: &str, mode: &str) -> NewProvider {
        NewProvider {
            id: id.to_string(),
            app_type: app_type.to_string(),
            name: format!("provider-{}", id),
            mode: mode.to_string(),
            settings_config: "{}".to_string(),
            category: None,
            sort_index: None,
            notes: None,
            meta: "{}".to_string(),
        }
    }

    #[test]
    fn switch_success_sets_current() {
        let db = setup();
        providers::create(&db, new_provider("a", "claude-code", "proxy")).unwrap();
        providers::create(&db, new_provider("b", "claude-code", "proxy")).unwrap();
        providers::set_current(&db, "a").unwrap();

        // 接管成功（注入返回 Ok）→ current 切到 b。
        let resp = perform_switch(&db, "b", |_p, _prev, _t| Ok(Vec::new())).unwrap();
        assert!(resp.warnings.is_empty());
        assert_eq!(
            providers::get_current(&db, "claude-code")
                .unwrap()
                .unwrap()
                .id,
            "b"
        );
    }

    #[test]
    fn switch_rollback_restores_previous_current() {
        let db = setup();
        providers::create(&db, new_provider("a", "claude-code", "proxy")).unwrap();
        providers::create(&db, new_provider("b", "claude-code", "proxy")).unwrap();
        providers::set_current(&db, "a").unwrap();

        // 接管失败 → is_current 必须回滚到 a。
        let err = perform_switch(&db, "b", |_p, _prev, _t| Err("boom".to_string())).unwrap_err();
        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            providers::get_current(&db, "claude-code")
                .unwrap()
                .unwrap()
                .id,
            "a",
            "接管失败后 current 应回滚到切换前"
        );
        assert!(!providers::get(&db, "b").unwrap().unwrap().is_current);
    }

    #[test]
    fn switch_rollback_clears_when_no_previous_current() {
        let db = setup();
        providers::create(&db, new_provider("b", "claude-code", "proxy")).unwrap();
        // 切换前无 current，接管失败后应清空（无悬挂 current）。
        let _ = perform_switch(&db, "b", |_p, _prev, _t| Err("boom".to_string())).unwrap_err();
        assert!(providers::get_current(&db, "claude-code")
            .unwrap()
            .is_none());
    }

    #[test]
    fn switch_missing_provider_returns_404() {
        let db = setup();
        let err = perform_switch(&db, "ghost", |_p, _prev, _t| Ok(Vec::new())).unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[test]
    fn switch_unsupported_app_type_returns_400() {
        let db = setup();
        providers::create(&db, new_provider("o", "opencode", "proxy")).unwrap();
        let err = perform_switch(&db, "o", |_p, _prev, _t| Ok(Vec::new())).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn switch_crypto_unavailable_maps_503() {
        let db = setup();
        providers::create(&db, new_provider("a", "claude-code", "proxy")).unwrap();
        providers::create(&db, new_provider("b", "claude-code", "direct")).unwrap();
        providers::set_current(&db, "a").unwrap();

        let err = perform_switch(&db, "b", |_p, _prev, _t| {
            Err("加密服务不可用，无法解密 direct 凭据".to_string())
        })
        .unwrap_err();
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
        // 依旧回滚。
        assert_eq!(
            providers::get_current(&db, "claude-code")
                .unwrap()
                .unwrap()
                .id,
            "a"
        );
    }

    #[test]
    fn switch_real_direct_missing_endpoint_rolls_back() {
        // 真实接管接线：direct provider 引用不存在的端点 → enable_direct 在写文件前失败 →
        // 回滚 current。config_dir 解析到真实 home 但不会写入（解析阶段即失败）。
        let db = setup();
        providers::create(&db, new_provider("a", "claude-code", "proxy")).unwrap();
        providers::create(
            &db,
            NewProvider {
                id: "b".to_string(),
                app_type: "claude-code".to_string(),
                name: "direct-b".to_string(),
                mode: "direct".to_string(),
                settings_config: r#"{"endpoint_id":"does-not-exist"}"#.to_string(),
                category: None,
                sort_index: None,
                notes: None,
                meta: "{}".to_string(),
            },
        )
        .unwrap();
        providers::set_current(&db, "a").unwrap();

        let data_dir = std::env::temp_dir();
        let err = perform_switch(&db, "b", |provider, _prev, tool| {
            tool_takeover::enable_direct(&db, tool, &data_dir, provider, None).map(|()| Vec::new())
        })
        .unwrap_err();
        // crypto=None 前会先命中端点不存在或 crypto 不可用，两者都触发回滚。
        assert!(
            err.0 == StatusCode::INTERNAL_SERVER_ERROR || err.0 == StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            providers::get_current(&db, "claude-code")
                .unwrap()
                .unwrap()
                .id,
            "a",
            "真实接管失败后 current 应回滚"
        );
    }
}
