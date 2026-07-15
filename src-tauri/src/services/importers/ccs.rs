//! 从本地 cc-switch (ccs) 一键同步 Claude 上游渠道。
//!
//! ccs 有两种存储格式，本模块统一抽象为 [`CcsSourceProvider`] 屏蔽差异：
//! - **新版（main 分支）**：`~/.cc-switch/cc-switch.db` SQLite，`providers` 表存
//!   `app_type='claude'` 的行，`settings_config` 字段是完整 Claude Code
//!   `settings.json`（明文内联 env 凭据）。
//! - **旧版（tauri-migration 分支）**：`~/.cc-switch/config.json` 文件，扁平
//!   `providers` map，`settingsConfig` 字段同上。
//!
//! detect/sync 都先探 SQLite（存在则用），不存在再探 config.json；两者都
//! 不存在 → detect `found=false` / sync Err。SQLite 只读打开，绝不写入。
//!
//! 与旧实现（endpoint + 加密双表）不同：当前基线是 ccs-native，provider 的
//! `settings_config` 直接内联存完整 Claude 配置，所以落库为近 1:1 复制——
//! 无 endpoint、无加密层。来源标记落在强类型 `ProviderMeta.imported_from /
//! imported_original_id`，用于幂等关联。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::provider::normalize_claude_models_in_value;
use crate::store::AppState;

/// 导入来源标识（`ProviderMeta.imported_from` 的取值）。
const IMPORTED_FROM_CCS: &str = "ccs";

// ── ccs 数据模型（仅反序列化所需字段）──────────────────────────────

/// ccs `~/.cc-switch/config.json` 顶层结构（扁平两字段，无 version/category 包裹）。
#[derive(Debug, Deserialize, Default)]
struct CcsConfig {
    /// `id → Provider` 的扁平 map（非数组）。
    #[serde(default)]
    providers: HashMap<String, CcsProvider>,
    /// 当前激活的 provider id，空串表示无。
    #[serde(default)]
    #[allow(dead_code)]
    current: String,
}

/// ccs 旧版 `Provider`（config.json 源），仅 4 字段。
#[derive(Debug, Deserialize)]
struct CcsProvider {
    id: String,
    name: String,
    /// 完整 Claude Code `settings.json`，明文内联 env 凭据。
    #[serde(rename = "settingsConfig")]
    settings_config: Value,
    #[serde(rename = "websiteUrl")]
    website_url: Option<String>,
}

/// 统一数据源 provider：屏蔽 SQLite 与 config.json 差异，detect/sync 后续逻辑
/// 只消费此结构。`category` 仅 SQLite 源有（config.json 源为 None）。
#[derive(Debug, Clone)]
pub struct CcsSourceProvider {
    pub id: String,
    pub name: String,
    /// 完整 Claude Code `settings.json`（明文 env 凭据）。
    pub settings_config: Value,
    pub website_url: Option<String>,
    /// SQLite 源的 `category` 字段；config.json 源无此字段。
    pub category: Option<String>,
}

/// 探测到的数据源标签，供前端展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcsSource {
    Sqlite,
    ConfigJson,
    None,
}

impl CcsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CcsSource::Sqlite => "sqlite",
            CcsSource::ConfigJson => "config.json",
            CcsSource::None => "none",
        }
    }
}

// ── detect / sync 公共契约（与前端共享）──────────────────────────

/// 单条 ccs provider 的探测结果（预览列表用，只读、不含明文凭据）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectItem {
    /// ccs 原 provider id。
    #[serde(rename = "originalId")]
    pub original_id: String,
    pub name: String,
    /// 从 `env.ANTHROPIC_BASE_URL` 提取，缺失/空则 `None`。
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// `env.ANTHROPIC_AUTH_TOKEN` 是否存在（不回传明文）。
    #[serde(rename = "hasApiKey")]
    pub has_api_key: bool,
    /// `env.ANTHROPIC_MODEL`（可选）。
    pub model: Option<String>,
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    /// 是否可导入：base_url 缺失/空 → false。
    pub importable: bool,
    /// 同步状态：`new`（新增）/ `update`（更新已导入）/ `unchanged`（无变化）。
    pub status: String,
    /// 落库最终名称（status=new 且与非 ccs 渠道同名 → 加后缀）。
    #[serde(rename = "importedName")]
    pub imported_name: String,
    /// status=update/unchanged 时，本地目标 provider 的 id。
    #[serde(rename = "targetProviderId", skip_serializing_if = "Option::is_none")]
    pub target_provider_id: Option<String>,
    /// 不可导入原因（如「无 base_url」），可导入时为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// detect 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectResponse {
    /// 实际读取的 ccs 数据源路径（字符串化便于前端展示）。
    #[serde(rename = "configPath")]
    pub config_path: String,
    /// 数据源：`"sqlite"` / `"config.json"` / `"none"`。
    pub source: String,
    /// 数据源是否存在；false 时 `providers` 为空。
    pub found: bool,
    pub providers: Vec<DetectItem>,
}

/// sync 请求单项：仅传 `original_id` 定位 ccs provider + `imported_name`（含冲突后缀）。
#[derive(Debug, Clone, Deserialize)]
pub struct ImportItem {
    #[serde(rename = "originalId")]
    pub original_id: String,
    #[serde(rename = "importedName")]
    pub imported_name: String,
}

/// sync 成功项（新增或更新）。
#[derive(Debug, Clone, Serialize)]
pub struct SyncedProvider {
    #[serde(rename = "originalId")]
    pub original_id: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub name: String,
}

/// sync 响应：逐项独立，单个失败记入 errors，其余继续。
#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncResponse {
    pub created: Vec<SyncedProvider>,
    pub updated: Vec<SyncedProvider>,
    pub skipped: Vec<SyncSkip>,
    pub errors: Vec<SyncError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncSkip {
    #[serde(rename = "originalId")]
    pub original_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncError {
    #[serde(rename = "originalId")]
    pub original_id: String,
    pub message: String,
}

// ── 路径解析 ─────────────────────────────────────────────────────

/// 解析 ccs config.json 路径：显式参数优先，否则 `dirs::home_dir()/.cc-switch/config.json`。
fn resolve_config_path(config_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = config_path {
        return Some(p);
    }
    dirs::home_dir().map(|h| h.join(".cc-switch").join("config.json"))
}

/// 解析 ccs SQLite db 路径：显式参数优先，否则 `dirs::home_dir()/.cc-switch/cc-switch.db`。
fn resolve_sqlite_path(sqlite_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = sqlite_path {
        return Some(p);
    }
    dirs::home_dir().map(|h| h.join(".cc-switch").join("cc-switch.db"))
}

// ── 数据源读取：SQLite + config.json → 统一 CcsSourceProvider ─────

/// 只读打开 ccs SQLite db，查 `app_type='claude'` 的 provider 列表。
///
/// 用 `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` 打开，避免锁冲突和误写。
/// db 不存在 → Ok(None)；打开/查询失败 → Err。
fn read_ccs_sqlite(path: &Path) -> Result<Option<Vec<CcsSourceProvider>>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("打开 cc-switch.db 失败: {}", e))?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, settings_config, website_url, category \
             FROM providers WHERE app_type = 'claude' ORDER BY id ASC",
        )
        .map_err(|e| format!("查询 ccs providers 失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let settings_json: String = row.get(2)?;
            let website_url: Option<String> = row.get(3)?;
            let category: Option<String> = row.get(4)?;
            let settings_config: Value = serde_json::from_str(&settings_json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(CcsSourceProvider {
                id,
                name,
                settings_config,
                website_url,
                category,
            })
        })
        .map_err(|e| format!("读取 ccs providers 行失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("解析 ccs provider 行失败: {}", e))?);
    }
    Ok(Some(out))
}

/// 读取并解析 ccs config.json；不存在 → Ok(None)；解析失败 → Err。
fn read_ccs_config(path: &Path) -> Result<Option<CcsConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 config.json 失败: {}", e))?;
    let cfg: CcsConfig =
        serde_json::from_str(&text).map_err(|e| format!("解析 config.json 失败: {}", e))?;
    Ok(Some(cfg))
}

/// 统一读取 ccs claude provider 列表 + 数据源标签 + 实际读取路径。
///
/// 优先探 SQLite db（新版 main 分支），存在则用；不存在再探 config.json（旧版
/// tauri-migration 分支）；两者都不存在 → 返回空 Vec + `CcsSource::None`。
/// SQLite 打开/查询失败 → Err（不回退 config.json，避免掩盖真实错误）。
///
/// `config_path` / `sqlite_path` 用于测试注入路径；生产调用传 None 走默认 home。
pub(crate) fn read_ccs_providers(
    config_path: Option<PathBuf>,
    sqlite_path: Option<PathBuf>,
) -> Result<(Vec<CcsSourceProvider>, CcsSource, PathBuf), String> {
    let sp = resolve_sqlite_path(sqlite_path);
    let cp = resolve_config_path(config_path);

    // 1. 先探 SQLite。
    if let Some(p) = sp.as_ref() {
        if p.exists() {
            let providers = read_ccs_sqlite(p)?.unwrap_or_default();
            return Ok((providers, CcsSource::Sqlite, p.clone()));
        }
    }
    // 2. 再探 config.json。
    if let Some(p) = cp.as_ref() {
        if p.exists() {
            if let Some(c) = read_ccs_config(p)? {
                // 按 id 升序保证输出稳定（HashMap 迭代顺序未定义）。
                let mut entries: Vec<&CcsProvider> = c.providers.values().collect();
                entries.sort_by(|a, b| a.id.cmp(&b.id));
                let providers = entries
                    .into_iter()
                    .map(|p| CcsSourceProvider {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        settings_config: p.settings_config.clone(),
                        website_url: p.website_url.clone(),
                        category: None,
                    })
                    .collect();
                return Ok((providers, CcsSource::ConfigJson, p.clone()));
            }
        }
    }
    // 3. 两者都不存在：返回展示用路径（优先 config.json，其次 SQLite，兜底虚拟路径）。
    let fallback = cp
        .or(sp)
        .unwrap_or_else(|| PathBuf::from("~/.cc-switch/config.json"));
    Ok((Vec::new(), CcsSource::None, fallback))
}

/// 从 settingsConfig.env 提取 ANTHROPIC_BASE_URL / AUTH_TOKEN / MODEL。
fn extract_env(settings: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let env = settings.get("env");
    let get = |k: &str| -> Option<String> {
        env.and_then(|e| e.get(k))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    };
    (
        get("ANTHROPIC_BASE_URL"),
        get("ANTHROPIC_AUTH_TOKEN"),
        get("ANTHROPIC_MODEL"),
    )
}

// ── 冲突重命名算法 ───────────────────────────────────────────────

/// 计算不与 `existing_names` 冲突的唯一名称。
///
/// 无冲突 → 原名；冲突 → `原名 (ccs)`；仍冲突 → `原名 (ccs 2)`、`原名 (ccs 3)` …
/// 兜底用 uuid 前缀（理论上极不可能触达）。
pub(crate) fn resolve_unique_name(desired: &str, existing_names: &HashSet<String>) -> String {
    if !existing_names.contains(desired) {
        return desired.to_string();
    }
    let base = format!("{} (ccs)", desired);
    if !existing_names.contains(&base) {
        return base;
    }
    for i in 2..=1000 {
        let cand = format!("{} (ccs {})", desired, i);
        if !existing_names.contains(&cand) {
            return cand;
        }
    }
    format!("{} (ccs {})", desired, uuid::Uuid::new_v4())
}

// ── settingsConfig 规范化相等判定 ────────────────────────────────

/// 规范化 JSON 后做相等判定，避免 key 顺序差异误判为「有变化」。
///
/// `serde_json::Value` 的 `Map` 默认按插入序，`==` 对象比较是逐 key 无序的，
/// 已能容忍顺序差异；此处额外把两侧都过一遍 Claude 模型规范化，保证与落库口径一致。
fn settings_equal(a: &Value, b: &Value) -> bool {
    let mut na = a.clone();
    let mut nb = b.clone();
    let _ = normalize_claude_models_in_value(&mut na);
    let _ = normalize_claude_models_in_value(&mut nb);
    na == nb
}

// ── 本地已导入索引 ───────────────────────────────────────────────

/// 本地已有 provider 的比对索引。
struct LocalIndex {
    /// 所有本地 claude provider 的 name 集合（用于冲突重命名）。
    names: HashSet<String>,
    /// `ccs original_id → (本地 provider id, 本地 settings_config)`，仅含来源=ccs 的项。
    imported: HashMap<String, (String, Value)>,
}

/// 收集本地 claude provider 的 name 集合与已导入(来源=ccs)的映射。
fn collect_local(state: &AppState) -> Result<LocalIndex, AppError> {
    let providers = state.db.get_all_providers(AppType::Claude.as_str())?;
    let mut names = HashSet::new();
    let mut imported = HashMap::new();
    for (id, p) in providers {
        names.insert(p.name.clone());
        if let Some(meta) = &p.meta {
            if meta.imported_from.as_deref() == Some(IMPORTED_FROM_CCS) {
                if let Some(orig) = &meta.imported_original_id {
                    imported.insert(orig.clone(), (id, p.settings_config.clone()));
                }
            }
        }
    }
    Ok(LocalIndex { names, imported })
}

// ── detect ───────────────────────────────────────────────────────

/// 探测 ccs 安装并返回预览列表（只读）。
///
/// 优先探 SQLite db，不存在再探 config.json。两者都不存在 → `found=false`
/// （非错误）；读取/解析失败 → Err。
pub fn detect(
    state: &AppState,
    config_path: Option<PathBuf>,
    sqlite_path: Option<PathBuf>,
) -> Result<DetectResponse, AppError> {
    let (source_providers, source, path) =
        read_ccs_providers(config_path, sqlite_path).map_err(AppError::Message)?;

    let found = source != CcsSource::None;
    if !found {
        return Ok(DetectResponse {
            config_path: path.to_string_lossy().to_string(),
            source: source.as_str().to_string(),
            found: false,
            providers: Vec::new(),
        });
    }

    let local = collect_local(state)?;
    // 预览期实时占用已分配的后缀名，避免批量内多个同名 ccs provider 互相撞名。
    let mut used_names = local.names.clone();

    let mut items: Vec<DetectItem> = Vec::new();
    for ccs_p in &source_providers {
        let (base_url, api_key, model) = extract_env(&ccs_p.settings_config);
        let (importable, warning) = match &base_url {
            None => (
                false,
                Some("无 base_url（官方登录渠道，无上游端点，无法导入）".to_string()),
            ),
            Some(_) => (true, None),
        };

        // 判定三态。
        let (status, imported_name, target_provider_id) =
            if let Some((local_id, local_settings)) = local.imported.get(&ccs_p.id) {
                if settings_equal(local_settings, &ccs_p.settings_config) {
                    (
                        "unchanged".to_string(),
                        ccs_p.name.clone(),
                        Some(local_id.clone()),
                    )
                } else {
                    (
                        "update".to_string(),
                        ccs_p.name.clone(),
                        Some(local_id.clone()),
                    )
                }
            } else {
                // 新增：若与本地非 ccs 渠道同名，加后缀。
                let imported_name = if used_names.contains(&ccs_p.name) {
                    let resolved = resolve_unique_name(&ccs_p.name, &used_names);
                    used_names.insert(resolved.clone());
                    resolved
                } else {
                    used_names.insert(ccs_p.name.clone());
                    ccs_p.name.clone()
                };
                ("new".to_string(), imported_name, None)
            };

        items.push(DetectItem {
            original_id: ccs_p.id.clone(),
            name: ccs_p.name.clone(),
            base_url,
            has_api_key: api_key.is_some(),
            model,
            website_url: ccs_p.website_url.clone(),
            importable,
            status,
            imported_name,
            target_provider_id,
            warning,
        });
    }

    Ok(DetectResponse {
        config_path: path.to_string_lossy().to_string(),
        source: source.as_str().to_string(),
        found: true,
        providers: items,
    })
}

// ── sync ─────────────────────────────────────────────────────────

/// 批量同步：每个勾选项按三态分派 new/update/unchanged；单项失败记入 errors，其余继续。
///
/// 落库直接走 DAO `save_provider`（不经 `ProviderService::add`），刻意规避 add 的
/// 「无当前渠道则自动设当前 + 写 live」副作用（D6/AC7）：
/// - new → INSERT，`is_current` 默认 0，不改当前渠道。
/// - update → UPDATE，DAO 保留 `is_current` / `in_failover_queue`，不写 `~/.claude/settings.json`。
///
/// 每项前重新读 ccs 源（不信任前端传的凭据/URL），先探 SQLite 再探 config.json。
pub fn sync(
    state: &AppState,
    config_path: Option<PathBuf>,
    sqlite_path: Option<PathBuf>,
    items: Vec<ImportItem>,
) -> Result<SyncResponse, AppError> {
    let (source_providers, source, _path) =
        read_ccs_providers(config_path, sqlite_path).map_err(AppError::Message)?;
    if source == CcsSource::None {
        return Err(AppError::Message(
            "ccs 数据源不存在（无 cc-switch.db 且无 config.json），无法同步".to_string(),
        ));
    }
    let by_id: HashMap<&str, &CcsSourceProvider> = source_providers
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let mut local = collect_local(state)?;
    let mut resp = SyncResponse::default();

    for item in items {
        let original_id = item.original_id.clone();
        match sync_one(state, &by_id, &mut local, item) {
            Ok(SyncOutcome::Created(p)) => resp.created.push(p),
            Ok(SyncOutcome::Updated(p)) => resp.updated.push(p),
            Ok(SyncOutcome::Skipped(reason)) => resp.skipped.push(SyncSkip {
                original_id,
                reason,
            }),
            Err(e) => resp.errors.push(SyncError {
                original_id,
                message: e.to_string(),
            }),
        }
    }

    Ok(resp)
}

enum SyncOutcome {
    Created(SyncedProvider),
    Updated(SyncedProvider),
    Skipped(String),
}

/// 处理单个同步项。`local` 会被就地更新（新增的 name / imported 映射），
/// 保证批量内后续项能感知前序项的落库结果（避免撞名、重复导入）。
fn sync_one(
    state: &AppState,
    by_id: &HashMap<&str, &CcsSourceProvider>,
    local: &mut LocalIndex,
    item: ImportItem,
) -> Result<SyncOutcome, AppError> {
    let Some(ccs_p) = by_id.get(item.original_id.as_str()).copied() else {
        return Ok(SyncOutcome::Skipped("ccs 中不存在该 id".to_string()));
    };

    let (base_url, _api_key, _model) = extract_env(&ccs_p.settings_config);
    if base_url.is_none() {
        return Ok(SyncOutcome::Skipped(
            "无 base_url（官方登录渠道，无法导入）".to_string(),
        ));
    }

    // 已导入过 → update 或 unchanged。
    if let Some((local_id, local_settings)) = local.imported.get(&ccs_p.id).cloned() {
        if settings_equal(&local_settings, &ccs_p.settings_config) {
            return Ok(SyncOutcome::Skipped("无变化".to_string()));
        }
        return update_provider(state, local, &local_id, ccs_p);
    }

    // 新增：确定最终名称（信任前端 imported_name，但再查一道本地重名）。
    let imported_name = if local.names.contains(&item.imported_name) {
        resolve_unique_name(&item.imported_name, &local.names)
    } else {
        item.imported_name.clone()
    };

    create_provider(state, local, &imported_name, ccs_p)
}

/// 新建 provider：内联 ccs settings_config，打来源标记，直接 INSERT。
fn create_provider(
    state: &AppState,
    local: &mut LocalIndex,
    imported_name: &str,
    ccs_p: &CcsSourceProvider,
) -> Result<SyncOutcome, AppError> {
    let provider_id = uuid::Uuid::new_v4().to_string();
    let mut settings = ccs_p.settings_config.clone();
    let _ = normalize_claude_models_in_value(&mut settings);

    if !settings.is_object() {
        return Err(AppError::localized(
            "provider.claude.settings.not_object",
            "Claude 配置必须是 JSON 对象",
            "Claude configuration must be a JSON object",
        ));
    }

    let mut provider = Provider::with_id(
        provider_id.clone(),
        imported_name.to_string(),
        settings.clone(),
        ccs_p.website_url.clone(),
    );
    provider.category = ccs_p
        .category
        .clone()
        .or_else(|| Some("custom".to_string()));
    provider.created_at = Some(chrono::Utc::now().timestamp_millis());
    let meta = provider.meta.get_or_insert_with(Default::default);
    meta.imported_from = Some(IMPORTED_FROM_CCS.to_string());
    meta.imported_original_id = Some(ccs_p.id.clone());

    state
        .db
        .save_provider(AppType::Claude.as_str(), &provider)?;

    // 登记，避免批量内后续项撞名/重复导入。
    local.names.insert(imported_name.to_string());
    local
        .imported
        .insert(ccs_p.id.clone(), (provider_id.clone(), settings));

    Ok(SyncOutcome::Created(SyncedProvider {
        original_id: ccs_p.id.clone(),
        provider_id,
        name: imported_name.to_string(),
    }))
}

/// 更新已导入 provider：读回既有行，仅覆盖 settings_config / website_url / 来源标记，
/// 保留其余字段（name / category / sort_index 等）后 save。
/// `save_provider` 的 UPDATE 会保留 is_current / in_failover_queue（D6/AC4/AC7）。
fn update_provider(
    state: &AppState,
    local: &mut LocalIndex,
    local_id: &str,
    ccs_p: &CcsSourceProvider,
) -> Result<SyncOutcome, AppError> {
    let mut provider = state
        .db
        .get_provider_by_id(local_id, AppType::Claude.as_str())?
        .ok_or_else(|| {
            AppError::Message(format!("本地目标渠道 {} 已不存在，跳过更新", local_id))
        })?;

    let mut settings = ccs_p.settings_config.clone();
    let _ = normalize_claude_models_in_value(&mut settings);
    if !settings.is_object() {
        return Err(AppError::localized(
            "provider.claude.settings.not_object",
            "Claude 配置必须是 JSON 对象",
            "Claude configuration must be a JSON object",
        ));
    }

    provider.settings_config = settings.clone();
    provider.website_url = ccs_p.website_url.clone();
    let meta = provider.meta.get_or_insert_with(Default::default);
    meta.imported_from = Some(IMPORTED_FROM_CCS.to_string());
    meta.imported_original_id = Some(ccs_p.id.clone());

    state
        .db
        .save_provider(AppType::Claude.as_str(), &provider)?;

    // 更新本地索引的 settings 快照，避免同一批次二次判定为 update。
    local
        .imported
        .insert(ccs_p.id.clone(), (local_id.to_string(), settings));

    Ok(SyncOutcome::Updated(SyncedProvider {
        original_id: ccs_p.id.clone(),
        provider_id: local_id.to_string(),
        name: provider.name,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::SystemTime;

    use serde_json::json;
    use tempfile::TempDir;

    use crate::database::Database;
    use crate::provider::ProviderMeta;

    fn state() -> AppState {
        AppState::new(Arc::new(Database::memory().expect("memory database")))
    }

    fn write_config(dir: &TempDir, providers: Vec<Value>) -> PathBuf {
        let providers = providers
            .into_iter()
            .map(|provider| {
                let id = provider["id"].as_str().expect("provider id").to_string();
                (id, provider)
            })
            .collect::<serde_json::Map<String, Value>>();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({ "providers": providers, "current": "" }))
                .expect("serialize fixture"),
        )
        .expect("write fixture");
        path
    }

    fn source_provider(id: &str, name: &str, base_url: Option<&str>, token: &str) -> Value {
        let mut env = serde_json::Map::new();
        if let Some(url) = base_url {
            env.insert("ANTHROPIC_BASE_URL".to_string(), json!(url));
        }
        env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(token));
        json!({
            "id": id,
            "name": name,
            "settingsConfig": { "env": env },
            "websiteUrl": "https://example.com"
        })
    }

    fn source_paths(dir: &TempDir, config_path: PathBuf) -> (Option<PathBuf>, Option<PathBuf>) {
        (
            Some(config_path),
            Some(dir.path().join("missing-cc-switch.db")),
        )
    }

    fn imported_provider(id: &str, name: &str, original_id: &str, token: &str) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            name.to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.example.com",
                    "ANTHROPIC_AUTH_TOKEN": token
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            imported_from: Some(IMPORTED_FROM_CCS.to_string()),
            imported_original_id: Some(original_id.to_string()),
            ..ProviderMeta::default()
        });
        provider
    }

    #[test]
    fn provider_meta_source_markers_roundtrip_and_omit_when_missing() {
        let meta = ProviderMeta {
            imported_from: Some("ccs".to_string()),
            imported_original_id: Some("source-id".to_string()),
            ..ProviderMeta::default()
        };
        let value = serde_json::to_value(&meta).expect("serialize meta");
        assert_eq!(value["importedFrom"], "ccs");
        assert_eq!(value["importedOriginalId"], "source-id");

        let decoded: ProviderMeta = serde_json::from_value(value).expect("deserialize meta");
        assert_eq!(decoded.imported_from.as_deref(), Some("ccs"));
        assert_eq!(decoded.imported_original_id.as_deref(), Some("source-id"));

        let empty = serde_json::to_value(ProviderMeta::default()).expect("serialize default meta");
        assert!(empty.get("importedFrom").is_none());
        assert!(empty.get("importedOriginalId").is_none());
    }

    #[test]
    fn read_prefers_sqlite_over_config_json() {
        let dir = TempDir::new().expect("temp dir");
        let config_path = write_config(
            &dir,
            vec![source_provider(
                "config-provider",
                "Config Provider",
                Some("https://config.example.com"),
                "config-token",
            )],
        );
        let sqlite_path = dir.path().join("cc-switch.db");
        let conn = Connection::open(&sqlite_path).expect("create fixture sqlite");
        conn.execute_batch(
            "CREATE TABLE providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                website_url TEXT,
                category TEXT
            );",
        )
        .expect("create providers table");
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, website_url, category)
             VALUES (?1, 'claude', ?2, ?3, NULL, 'custom')",
            rusqlite::params![
                "sqlite-provider",
                "SQLite Provider",
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://sqlite.example.com",
                        "ANTHROPIC_AUTH_TOKEN": "sqlite-token"
                    }
                })
                .to_string()
            ],
        )
        .expect("insert sqlite provider");
        drop(conn);

        let (providers, source, path) =
            read_ccs_providers(Some(config_path), Some(sqlite_path.clone())).expect("read source");
        assert_eq!(source, CcsSource::Sqlite);
        assert_eq!(path, sqlite_path);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "sqlite-provider");
    }

    #[test]
    fn read_falls_back_to_config_and_reports_bad_json() {
        let dir = TempDir::new().expect("temp dir");
        let config_path = write_config(
            &dir,
            vec![source_provider(
                "config-provider",
                "Config Provider",
                Some("https://config.example.com"),
                "token",
            )],
        );
        let (providers, source, _) = read_ccs_providers(
            Some(config_path.clone()),
            Some(dir.path().join("missing.db")),
        )
        .expect("read config source");
        assert_eq!(source, CcsSource::ConfigJson);
        assert_eq!(providers[0].id, "config-provider");

        std::fs::write(&config_path, b"{broken").expect("write broken json");
        let err = read_ccs_providers(Some(config_path), Some(dir.path().join("still-missing.db")))
            .expect_err("bad json must fail");
        assert!(err.contains("解析 config.json 失败"));
    }

    #[test]
    fn detect_reports_not_found_without_error() {
        let dir = TempDir::new().expect("temp dir");
        let state = state();
        let response = detect(
            &state,
            Some(dir.path().join("missing-config.json")),
            Some(dir.path().join("missing.db")),
        )
        .expect("not found is not an error");
        assert!(!response.found);
        assert_eq!(response.source, "none");
        assert!(response.providers.is_empty());
    }

    #[test]
    fn detect_computes_new_update_unchanged_and_name_suffix() {
        let dir = TempDir::new().expect("temp dir");
        let state = state();
        let app = AppType::Claude.as_str();

        let manual = Provider::with_id(
            "manual".to_string(),
            "Duplicate".to_string(),
            json!({ "env": {} }),
            None,
        );
        state.db.save_provider(app, &manual).expect("save manual");
        state
            .db
            .save_provider(
                app,
                &imported_provider("same", "Same Local", "same-source", "same-token"),
            )
            .expect("save unchanged imported");
        state
            .db
            .save_provider(
                app,
                &imported_provider("changed", "Changed Local", "changed-source", "old-token"),
            )
            .expect("save changed imported");

        let config_path = write_config(
            &dir,
            vec![
                source_provider(
                    "new-source",
                    "Duplicate",
                    Some("https://api.example.com"),
                    "new-token",
                ),
                source_provider(
                    "same-source",
                    "Same Source",
                    Some("https://api.example.com"),
                    "same-token",
                ),
                source_provider(
                    "changed-source",
                    "Changed Source",
                    Some("https://api.example.com"),
                    "new-token",
                ),
                source_provider("no-base", "Official Login", None, "token"),
            ],
        );
        let (config, sqlite) = source_paths(&dir, config_path);
        let response = detect(&state, config, sqlite).expect("detect");
        let by_id = response
            .providers
            .iter()
            .map(|item| (item.original_id.as_str(), item))
            .collect::<HashMap<_, _>>();

        assert_eq!(by_id["new-source"].status, "new");
        assert_eq!(by_id["new-source"].imported_name, "Duplicate (ccs)");
        assert_eq!(by_id["same-source"].status, "unchanged");
        assert_eq!(
            by_id["same-source"].target_provider_id.as_deref(),
            Some("same")
        );
        assert_eq!(by_id["changed-source"].status, "update");
        assert_eq!(
            by_id["changed-source"].target_provider_id.as_deref(),
            Some("changed")
        );
        assert!(!by_id["no-base"].importable);
        assert!(by_id["no-base"].warning.is_some());
    }

    #[test]
    fn sync_is_idempotent_and_keeps_source_file_read_only() {
        let dir = TempDir::new().expect("temp dir");
        let state = state();
        let config_path = write_config(
            &dir,
            vec![source_provider(
                "source-1",
                "Imported",
                Some("https://api.example.com"),
                "secret",
            )],
        );
        let before = std::fs::read(&config_path).expect("read before");
        let before_modified = std::fs::metadata(&config_path)
            .expect("metadata before")
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let item = ImportItem {
            original_id: "source-1".to_string(),
            imported_name: "Imported".to_string(),
        };

        let (config, sqlite) = source_paths(&dir, config_path.clone());
        let first = sync(&state, config, sqlite, vec![item.clone()]).expect("first sync");
        assert_eq!(first.created.len(), 1);
        assert!(first.updated.is_empty());
        assert!(first.errors.is_empty());

        let (config, sqlite) = source_paths(&dir, config_path.clone());
        let second = sync(&state, config, sqlite, vec![item]).expect("second sync");
        assert!(second.created.is_empty());
        assert!(second.updated.is_empty());
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(second.skipped[0].reason, "无变化");
        assert_eq!(
            state
                .db
                .get_all_providers(AppType::Claude.as_str())
                .expect("list")
                .len(),
            1
        );

        assert_eq!(std::fs::read(&config_path).expect("read after"), before);
        let after_modified = std::fs::metadata(&config_path)
            .expect("metadata after")
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        assert_eq!(after_modified, before_modified);
    }

    #[test]
    fn update_preserves_current_failover_sort_and_local_name() {
        let dir = TempDir::new().expect("temp dir");
        let state = state();
        let app = AppType::Claude.as_str();
        let mut existing = imported_provider("local-id", "Local Name", "source-id", "old-token");
        existing.in_failover_queue = true;
        existing.sort_index = Some(42);
        existing.notes = Some("local note".to_string());
        state
            .db
            .save_provider(app, &existing)
            .expect("save existing");
        state
            .db
            .set_current_provider(app, "local-id")
            .expect("set current");

        let config_path = write_config(
            &dir,
            vec![source_provider(
                "source-id",
                "Source Rename",
                Some("https://api.example.com"),
                "new-token",
            )],
        );
        let (config, sqlite) = source_paths(&dir, config_path);
        let response = sync(
            &state,
            config,
            sqlite,
            vec![ImportItem {
                original_id: "source-id".to_string(),
                imported_name: "Source Rename".to_string(),
            }],
        )
        .expect("update sync");
        assert_eq!(response.updated.len(), 1);
        assert!(response.errors.is_empty());

        let updated = state
            .db
            .get_provider_by_id("local-id", app)
            .expect("get provider")
            .expect("provider exists");
        assert_eq!(updated.name, "Local Name");
        assert_eq!(updated.sort_index, Some(42));
        assert_eq!(updated.notes.as_deref(), Some("local note"));
        assert!(updated.in_failover_queue);
        assert_eq!(
            updated.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
            "new-token"
        );
        assert_eq!(
            state
                .db
                .get_current_provider(app)
                .expect("get current")
                .as_deref(),
            Some("local-id")
        );
    }

    #[test]
    fn one_item_failure_does_not_block_following_items() {
        let dir = TempDir::new().expect("temp dir");
        let state = state();
        {
            let conn = state.db.conn.lock().expect("lock db");
            conn.execute_batch(
                "CREATE TRIGGER reject_bad_ccs_provider
                 BEFORE INSERT ON providers
                 WHEN NEW.name = 'Bad'
                 BEGIN
                    SELECT RAISE(ABORT, 'forced item failure');
                 END;",
            )
            .expect("create failure trigger");
        }

        let config_path = write_config(
            &dir,
            vec![
                source_provider(
                    "bad-source",
                    "Bad",
                    Some("https://bad.example.com"),
                    "bad-token",
                ),
                source_provider(
                    "good-source",
                    "Good",
                    Some("https://good.example.com"),
                    "good-token",
                ),
            ],
        );
        let (config, sqlite) = source_paths(&dir, config_path);
        let response = sync(
            &state,
            config,
            sqlite,
            vec![
                ImportItem {
                    original_id: "bad-source".to_string(),
                    imported_name: "Bad".to_string(),
                },
                ImportItem {
                    original_id: "good-source".to_string(),
                    imported_name: "Good".to_string(),
                },
            ],
        )
        .expect("batch returns partial result");

        assert_eq!(response.errors.len(), 1);
        assert_eq!(response.errors[0].original_id, "bad-source");
        assert_eq!(response.created.len(), 1);
        assert_eq!(response.created[0].original_id, "good-source");
    }
}
