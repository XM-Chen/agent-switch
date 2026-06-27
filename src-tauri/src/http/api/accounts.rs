use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::accounts::{self, AccountRow, AccountUpdate, NewAccount};

/// 账号脱敏响应（不含凭据）。
#[derive(Serialize)]
pub struct AccountResponse {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub platform: String,
    pub status: String,
    pub priority: i64,
    pub has_credentials: bool,
    pub last_login_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<AccountRow> for AccountResponse {
    fn from(r: AccountRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            account_type: r.account_type,
            platform: r.platform,
            status: r.status,
            priority: r.priority,
            has_credentials: r.credentials_encrypted.is_some(),
            last_login_at: r.last_login_at,
            last_error: r.last_error,
            last_error_at: r.last_error_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// 创建账号请求（API Key 类型）。
#[derive(Deserialize)]
pub struct CreateAccountRequest {
    pub name: String,
    pub account_type: String,
    pub platform: String,
    pub api_key: Option<String>,
    pub priority: Option<i64>,
}

/// 更新账号请求。
#[derive(Deserialize)]
pub struct UpdateAccountRequest {
    pub name: Option<String>,
    pub status: Option<String>,
    pub api_key: Option<Option<String>>,
    pub priority: Option<i64>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AccountResponse>>, (StatusCode, String)> {
    let rows = accounts::list(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(AccountResponse::from).collect()))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let row = accounts::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match row {
        Some(r) => Ok(Json(AccountResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "账号不存在".to_string())),
    }
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AccountResponse>), (StatusCode, String)> {
    if req.account_type == "oauth_codex" {
        return Err((
            StatusCode::BAD_REQUEST,
            "OAuth Codex 账号请通过 /api/auth/codex/login 创建".to_string(),
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let credentials_encrypted = encrypt_api_key(&state, &id, req.api_key.as_deref())?;

    let new = NewAccount {
        id,
        name: req.name,
        account_type: req.account_type,
        platform: req.platform,
        credentials_encrypted,
        extra_json: None,
        priority: req.priority.unwrap_or(0),
    };
    let row =
        accounts::create(&state.db, new).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(AccountResponse::from(row))))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAccountRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut update = AccountUpdate {
        name: req.name,
        status: req.status,
        priority: req.priority,
        ..Default::default()
    };

    if let Some(opt) = req.api_key {
        // Some(None) => 清除凭据；Some(Some(key)) => 更新凭据。
        let encrypted: Option<Vec<u8>> = match opt {
            None => None,
            Some(key) => encrypt_api_key(&state, &id, Some(&key))?,
        };
        update.credentials_encrypted = Some(encrypted);
    }

    accounts::update(&state.db, &id, update).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    accounts::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// 加密 API Key 凭据，返回 BLOB。
fn encrypt_api_key(
    state: &AppState,
    id: &str,
    api_key: Option<&str>,
) -> Result<Option<Vec<u8>>, (StatusCode, String)> {
    let key = match api_key {
        None => None,
        Some(k) => {
            let crypto = state.crypto.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "系统凭据管理器不可用，无法保存凭据".to_string(),
            ))?;
            let json = serde_json::json!({ "api_key": k });
            let json_bytes = serde_json::to_vec(&json).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("序列化凭据失败: {}", e),
                )
            })?;
            Some(crypto.encrypt(&json_bytes, id.as_bytes()).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("加密失败: {}", e),
                )
            })?)
        }
    };
    Ok(key)
}
