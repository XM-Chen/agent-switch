//! Skills 管理 API（cc-skills）。
//!
//! 覆盖本地目录/zip 导入、GitHub 安装、列表、启用/禁用、手动 sync、status、冲突报告、
//! 卸载/备份/恢复、更新检查与批量更新、GitHub 发现搜索与未托管扫描。
//! 网络访问（install-repo/search/check-updates/update）仅在这些端点被显式调用时发生。
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::skills::SkillRow;
use crate::db::dao::{app_metadata, skills};
use crate::services::skills::{
    self as skill_service, ImportDirInput, SkillApp,
};

/// GitHub token 存储 key（app_metadata）。首版匿名可用，配置后提高限速与私仓可达。
const GITHUB_TOKEN_KEY: &str = "skills_github_token";

/// 读取可选 GitHub token；缺失或空返回 None（匿名访问）。
fn github_token(state: &AppState) -> Option<String> {
    app_metadata::get(&state.db, GITHUB_TOKEN_KEY)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
}

#[derive(Serialize)]
pub struct SkillResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub directory: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub repo_subdir: Option<String>,
    pub readme_url: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
    pub installed_at: String,
    pub updated_at: String,
    pub content_hash: String,
    pub created_at: String,
}

impl From<SkillRow> for SkillResponse {
    fn from(r: SkillRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            description: r.description,
            directory: r.directory,
            source_type: r.source_type,
            source_url: r.source_url,
            repo_owner: r.repo_owner,
            repo_name: r.repo_name,
            repo_branch: r.repo_branch,
            repo_subdir: r.repo_subdir,
            readme_url: r.readme_url,
            enabled_claude: r.enabled_claude,
            enabled_codex: r.enabled_codex,
            enabled_gemini: r.enabled_gemini,
            enabled_opencode: r.enabled_opencode,
            enabled_hermes: r.enabled_hermes,
            installed_at: r.installed_at,
            updated_at: r.updated_at,
            content_hash: r.content_hash,
            created_at: r.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct ImportDirRequest {
    pub source_path: String,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: Option<bool>,
    pub enabled_codex: Option<bool>,
    pub enabled_gemini: Option<bool>,
    pub enabled_opencode: Option<bool>,
    pub enabled_hermes: Option<bool>,
}

#[derive(Deserialize)]
pub struct SetEnabledRequest {
    pub enabled: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list))
        .route("/import-dir", post(import_dir))
        .route("/import-zip", post(import_zip))
        .route("/install-repo", post(install_repo))
        .route("/sync", post(sync_all))
        .route("/status", get(status))
        .route("/scan-unmanaged", post(scan_unmanaged))
        .route("/backups", get(list_backups))
        .route("/restore", post(restore))
        .route("/search", post(search))
        .route("/check-updates", post(check_updates))
        .route("/update", post(update))
        .route("/{id}", get(get_one))
        .route("/{id}", axum::routing::delete(uninstall))
        .route("/{id}/sync", post(sync_skill))
        .route("/{id}/{app}", post(set_enabled))
}

fn map_error(e: String) -> (StatusCode, String) {
    // GitHub 限速/访问受限归为 503（外部依赖暂不可用）。
    if e.contains("限速") || e.contains("受限") || e.contains("访问受限") {
        return (StatusCode::SERVICE_UNAVAILABLE, e);
    }
    if e.contains("不存在")
        || e.contains("不支持")
        || e.contains("必须")
        || e.contains("不能")
        || e.contains("拒绝")
        || e.contains("已存在")
        || e.contains("不是目录")
        || e.contains("符号链接")
        || e.contains("路径越界")
        || e.contains("为空")
    {
        (StatusCode::BAD_REQUEST, e)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}

fn parse_app_or_400(app: &str) -> Result<SkillApp, (StatusCode, String)> {
    skill_service::parse_app(app)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("不支持的 app: {}", app)))
}

async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SkillResponse>>, (StatusCode, String)> {
    let rows = skills::list(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(SkillResponse::from).collect()))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SkillResponse>, (StatusCode, String)> {
    match skills::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))? {
        Some(r) => Ok(Json(SkillResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "skill 不存在".to_string())),
    }
}

async fn import_dir(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportDirRequest>,
) -> Result<(StatusCode, Json<skill_service::ImportReport>), (StatusCode, String)> {
    if req.source_path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "源目录不能为空".to_string()));
    }
    let report = skill_service::import_dir(
        &state.db,
        &state.data_dir,
        ImportDirInput {
            source_path: PathBuf::from(req.source_path),
            directory: req.directory,
            name: req.name,
            description: req.description,
            enabled_claude: req.enabled_claude.unwrap_or(false),
            enabled_codex: req.enabled_codex.unwrap_or(false),
            enabled_gemini: req.enabled_gemini.unwrap_or(false),
            enabled_opencode: req.enabled_opencode.unwrap_or(false),
            enabled_hermes: req.enabled_hermes.unwrap_or(false),
        },
    )
    .map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(report)))
}

async fn set_enabled(
    State(state): State<Arc<AppState>>,
    Path((id, app)): Path<(String, String)>,
    Json(req): Json<SetEnabledRequest>,
) -> Result<Json<skill_service::SyncReport>, (StatusCode, String)> {
    let app = parse_app_or_400(&app)?;
    let report = skill_service::set_enabled(&state.db, &state.data_dir, &id, app, req.enabled)
        .map_err(map_error)?;
    Ok(Json(report))
}

async fn sync_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<skill_service::SyncReport>>, (StatusCode, String)> {
    let report = skill_service::sync_all(&state.db, &state.data_dir).map_err(map_error)?;
    Ok(Json(report))
}

async fn sync_skill(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<skill_service::SyncReport>>, (StatusCode, String)> {
    let row = skills::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "skill 不存在".to_string()))?;
    let mut out = Vec::new();
    for app in SkillApp::all() {
        if app_enabled(app, &row) {
            out.push(skill_service::sync_one(&state.db, &state.data_dir, app).map_err(map_error)?);
        }
    }
    Ok(Json(out))
}

fn app_enabled(app: SkillApp, row: &SkillRow) -> bool {
    match app {
        SkillApp::Claude => row.enabled_claude,
        SkillApp::Codex => row.enabled_codex,
        SkillApp::Gemini => row.enabled_gemini,
        SkillApp::OpenCode => row.enabled_opencode,
        SkillApp::Hermes => row.enabled_hermes,
    }
}

async fn status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<skill_service::SkillStatus>, (StatusCode, String)> {
    let status = skill_service::status(&state.db, &state.data_dir).map_err(map_error)?;
    Ok(Json(status))
}

// ---- 阶段 C：GitHub 安装 / zip 导入 / 扫描 / 备份恢复 / 发现 / 更新 ----

#[derive(Deserialize)]
pub struct InstallRepoRequest {
    pub repo: String,
    pub branch: Option<String>,
    pub subdir: Option<String>,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: Option<bool>,
    pub enabled_codex: Option<bool>,
    pub enabled_gemini: Option<bool>,
    pub enabled_opencode: Option<bool>,
    pub enabled_hermes: Option<bool>,
}

async fn install_repo(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InstallRepoRequest>,
) -> Result<(StatusCode, Json<skill_service::ImportReport>), (StatusCode, String)> {
    if req.repo.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "repo 不能为空".to_string()));
    }
    let token = github_token(&state);
    let report = skill_service::install_repo(
        &state.db,
        &state.data_dir,
        token,
        skill_service::InstallRepoInput {
            repo: req.repo,
            branch: req.branch,
            subdir: req.subdir,
            directory: req.directory,
            name: req.name,
            description: req.description,
            enabled_claude: req.enabled_claude.unwrap_or(false),
            enabled_codex: req.enabled_codex.unwrap_or(false),
            enabled_gemini: req.enabled_gemini.unwrap_or(false),
            enabled_opencode: req.enabled_opencode.unwrap_or(false),
            enabled_hermes: req.enabled_hermes.unwrap_or(false),
        },
    )
    .await
    .map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(report)))
}

#[derive(Deserialize)]
pub struct ImportZipRequest {
    pub zip_path: String,
    pub subdir: Option<String>,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: Option<bool>,
    pub enabled_codex: Option<bool>,
    pub enabled_gemini: Option<bool>,
    pub enabled_opencode: Option<bool>,
    pub enabled_hermes: Option<bool>,
}

async fn import_zip(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportZipRequest>,
) -> Result<(StatusCode, Json<skill_service::ImportReport>), (StatusCode, String)> {
    if req.zip_path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "zip 路径不能为空".to_string()));
    }
    let report = skill_service::import_zip(
        &state.db,
        &state.data_dir,
        skill_service::ImportZipInput {
            zip_path: PathBuf::from(req.zip_path),
            subdir: req.subdir,
            directory: req.directory,
            name: req.name,
            description: req.description,
            enabled_claude: req.enabled_claude.unwrap_or(false),
            enabled_codex: req.enabled_codex.unwrap_or(false),
            enabled_gemini: req.enabled_gemini.unwrap_or(false),
            enabled_opencode: req.enabled_opencode.unwrap_or(false),
            enabled_hermes: req.enabled_hermes.unwrap_or(false),
        },
    )
    .map_err(map_error)?;
    Ok((StatusCode::CREATED, Json(report)))
}

async fn scan_unmanaged(
    State(state): State<Arc<AppState>>,
) -> Result<Json<skill_service::ScanUnmanagedReport>, (StatusCode, String)> {
    let report =
        skill_service::scan_unmanaged(&state.db, &state.data_dir).map_err(map_error)?;
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct BackupsQuery {
    pub directory: Option<String>,
}

async fn list_backups(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<BackupsQuery>,
) -> Result<Json<Vec<skill_service::BackupEntry>>, (StatusCode, String)> {
    let entries =
        skill_service::list_backups(&state.data_dir, q.directory.as_deref()).map_err(map_error)?;
    Ok(Json(entries))
}

async fn uninstall(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<skill_service::UninstallReport>, (StatusCode, String)> {
    let report = skill_service::uninstall(&state.db, &state.data_dir, &id).map_err(map_error)?;
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct RestoreRequest {
    pub directory: String,
    pub timestamp: String,
}

async fn restore(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<skill_service::RestoreReport>, (StatusCode, String)> {
    let report = skill_service::restore(&state.db, &state.data_dir, &req.directory, &req.timestamp)
        .map_err(map_error)?;
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<skill_service::SearchReport>, (StatusCode, String)> {
    let token = github_token(&state);
    let report = skill_service::search(&req.query, token)
        .await
        .map_err(map_error)?;
    Ok(Json(report))
}

async fn check_updates(
    State(state): State<Arc<AppState>>,
) -> Result<Json<skill_service::CheckUpdatesReport>, (StatusCode, String)> {
    let token = github_token(&state);
    let report = skill_service::check_updates(&state.db, &state.data_dir, token, None)
        .await
        .map_err(map_error)?;
    Ok(Json(report))
}

#[derive(Deserialize)]
pub struct UpdateRequest {
    /// 为空或缺省时更新所有 GitHub 来源 skill。
    pub ids: Option<Vec<String>>,
}

async fn update(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpdateRequest>,
) -> Result<Json<skill_service::UpdateReport>, (StatusCode, String)> {
    let token = github_token(&state);
    let report = skill_service::update(&state.db, &state.data_dir, token, req.ids)
        .await
        .map_err(map_error)?;
    Ok(Json(report))
}
