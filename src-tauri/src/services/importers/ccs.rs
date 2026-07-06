//! 从本地 cc-switch (ccs) 一键导入 Claude 上游渠道。
//!
//! ccs 有两种存储格式，本模块统一抽象为 [`CcsSourceProvider`] 屏蔽差异：
//! - **新版（main 分支）**：`~/.cc-switch/cc-switch.db` SQLite，`providers` 表存
//!   `app_type='claude'` 的行，`settings_config` 字段是完整 Claude Code
//!   `settings.json`（明文内联 env 凭据）。
//! - **旧版（tauri-migration 分支）**：`~/.cc-switch/config.json` 文件，扁平
//!   `providers` map，`settingsConfig` 字段同上。
//!
//! detect/import 都先探 SQLite（存在则用），不存在再探 config.json；两者都
//! 不存在 → detect `found=false` / import Err。SQLite 只读打开，不写入。
//!
//! agent-switch 的 direct 模式刻意偏离 ccs 明文内联：provider 的
//! `settings_config` 只引用 `endpoint_id`，真实 `base_url` 与加密后的 token 落在
//! `endpoints` 表。因此导入 ccs 渠道需要为每条数据建两行：endpoint（加密 token）
//! + direct provider（引用 endpoint_id，meta 记录来源）。
//!
//! 详见 `.trellis/tasks/07-06-import-from-ccs/{prd,design}.md` 与
//! `research/{ccs-data-format,agent-switch-direct-provider-path}.md`。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::dao::endpoints;
use crate::db::dao::providers;
use crate::services::crypto::CryptoService;

// ── ccs 数据模型（仅反序列化所需字段）──────────────────────────────

/// ccs `~/.cc-switch/config.json` 顶层结构（扁平两字段，无 version/category 包裹）。
///
/// `providers` 是 `id → Provider` 的 map；`current` 是某 provider id 或空串。
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
///
/// ccs 用 camelCase（`settingsConfig`/`websiteUrl`），这里 rename 对齐。
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

/// 统一数据源 provider：屏蔽 SQLite 与 config.json 差异，detect/import 后续逻辑
/// 只消费此结构。`category` 仅 SQLite 源有（config.json 源为 None）。
#[derive(Debug, Clone)]
pub struct CcsSourceProvider {
    pub id: String,
    pub name: String,
    /// 完整 Claude Code `settings.json`（明文 env 凭据）。
    pub settings_config: Value,
    pub website_url: Option<String>,
    /// SQLite 源的 `category` 字段；config.json 源无此字段。
    #[allow(dead_code)]
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

// ── detect / import 公共契约（与 HTTP 层及前端共享）─────────────────

/// 单条 ccs provider 的探测结果（预览列表用，只读、不含明文凭据）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectItem {
    /// ccs 原 provider id。
    pub original_id: String,
    pub name: String,
    /// 从 `env.ANTHROPIC_BASE_URL` 提取，缺失/空则 `None`。
    pub base_url: Option<String>,
    /// `env.ANTHROPIC_AUTH_TOKEN` 是否存在（不回传明文）。
    pub has_api_key: bool,
    /// `env.ANTHROPIC_MODEL`（可选）。
    pub model: Option<String>,
    pub website_url: Option<String>,
    /// 是否可导入：base_url 缺失/空 → false。
    pub importable: bool,
    /// 与本地已有 provider 同名 → true，`imported_name` 为加后缀后的名称。
    pub conflict: bool,
    /// 加后缀后的最终名称（无冲突时等于原 name）。
    pub imported_name: String,
    /// 本地已有 `meta.original_id` 匹配的 provider → true。
    pub already_imported: bool,
    /// 不可导入原因（如「无 base_url」），可导入时为 None。
    pub warning: Option<String>,
}

/// detect 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectResponse {
    /// 实际读取的 ccs 数据源路径（SQLite db 或 config.json，字符串化便于前端展示）。
    pub config_path: String,
    /// 数据源：`"sqlite"` / `"config.json"` / `"none"`。
    pub source: String,
    /// 数据源是否存在；false 时 `providers` 为空。
    pub found: bool,
    pub providers: Vec<DetectItem>,
}

/// import 请求单项：仅传 `original_id` 定位 ccs provider + `imported_name`（含冲突后缀）。
#[derive(Debug, Clone, Deserialize)]
pub struct ImportItem {
    pub original_id: String,
    pub imported_name: String,
}

/// import 成功项。
#[derive(Debug, Clone, Serialize)]
pub struct ImportedProvider {
    pub original_id: String,
    pub provider_id: String,
    pub endpoint_id: String,
    pub name: String,
}

/// import 响应：逐项独立，单个失败记入 errors，其余继续。
#[derive(Debug, Clone, Serialize)]
pub struct ImportResponse {
    pub created_providers: Vec<ImportedProvider>,
    pub skipped: Vec<ImportSkip>,
    pub errors: Vec<ImportError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportSkip {
    pub original_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportError {
    pub original_id: String,
    pub message: String,
}

// ── 路径解析 ─────────────────────────────────────────────────────

/// 解析 ccs config.json 路径：显式参数优先，否则 `dirs::home_dir()/.cc-switch/config.json`。
///
/// `dirs::home_dir()` 在某些环境可能返回 None，此时返回 None（上层据此返回
/// `found=false`）。
fn resolve_config_path(config_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = config_path {
        return Some(p);
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        dirs::home_dir().map(|h| h.join(".cc-switch").join("config.json"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// 解析 ccs SQLite db 路径：显式参数优先，否则 `dirs::home_dir()/.cc-switch/cc-switch.db`。
fn resolve_sqlite_path(sqlite_path: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = sqlite_path {
        return Some(p);
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        dirs::home_dir().map(|h| h.join(".cc-switch").join("cc-switch.db"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
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
            // settings_config 是 Claude Code settings.json 全文，解析为 Value。
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
    // 先解析两条候选路径（取所有权），避免后续 fallback 再用一次已 move 的值。
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
            let cfg = read_ccs_config(p)?;
            if let Some(c) = cfg {
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
    // 3. 两者都不存在：返回一个用于展示的路径（优先 config.json，其次 SQLite，
    //    兜底虚拟路径），source=none 由调用方据此返回 found=false。
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

// ── 冲突重命名算法（design 第 4 节）──────────────────────────────

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
    // 兜底：理论不可达，1000 个候选都撞名时用 uuid 保证唯一。
    format!("{} (ccs {})", desired, uuid::Uuid::new_v4())
}

// ── 本地比对辅助 ─────────────────────────────────────────────────

/// 解析 provider.meta JSON，取 `imported_from` 与 `original_id` 字段。
fn parse_meta(meta: &str) -> (Option<String>, Option<String>) {
    let v: Value = serde_json::from_str(meta).unwrap_or(Value::Null);
    (
        v.get("imported_from")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        v.get("original_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    )
}

/// 收集本地 claude-code provider 的 name 集合与已导入的 original_id 集合。
fn collect_local(db: &Mutex<Connection>) -> Result<(HashSet<String>, HashSet<String>), String> {
    let rows = providers::list_by_app(db, "claude-code")?;
    let mut names = HashSet::new();
    let mut imported_ids = HashSet::new();
    for r in rows {
        names.insert(r.name);
        let (from, orig) = parse_meta(&r.meta);
        if from.as_deref() == Some("ccs") {
            if let Some(o) = orig {
                imported_ids.insert(o);
            }
        }
    }
    Ok((names, imported_ids))
}

// ── detect ───────────────────────────────────────────────────────

/// 探测 ccs 安装并返回预览列表（只读）。
///
/// 优先探 SQLite db（`sqlite_path` 或默认 `~/.cc-switch/cc-switch.db`），不存在
/// 再探 config.json（`config_path` 或默认 `~/.cc-switch/config.json`）。两者都
/// 不存在 → `found=false`（非错误）；读取/解析失败 → Err。
pub fn detect(
    db: &Mutex<Connection>,
    config_path: Option<PathBuf>,
    sqlite_path: Option<PathBuf>,
) -> Result<DetectResponse, String> {
    let (source_providers, source, path) = read_ccs_providers(config_path, sqlite_path)?;

    let found = !source_providers.is_empty() || source != CcsSource::None;
    if !found {
        return Ok(DetectResponse {
            config_path: path.to_string_lossy().to_string(),
            source: source.as_str().to_string(),
            found: false,
            providers: Vec::new(),
        });
    }

    let (mut local_names, imported_ids) = collect_local(db)?;

    // read_ccs_providers 已按 id 升序返回，输出稳定。
    let mut items: Vec<DetectItem> = Vec::new();

    for ccs_p in &source_providers {
        let (base_url, api_key, model) = extract_env(&ccs_p.settings_config);
        let already_imported = imported_ids.contains(&ccs_p.id);

        let (importable, warning) = match &base_url {
            None => (
                false,
                Some("无 base_url（官方登录渠道，无上游端点，无法导入）".to_string()),
            ),
            Some(_) => (true, None),
        };

        // 冲突重命名：仅当未被 already_imported 占用时才需要新名（已导入项默认
        // 不勾选，但仍计算 imported_name 供前端展示）。
        let conflict = local_names.contains(&ccs_p.name) && !already_imported;
        let imported_name = if conflict {
            let resolved = resolve_unique_name(&ccs_p.name, &local_names);
            // 占位，避免批量预览内多个同名 ccs provider 互相撞名。
            local_names.insert(resolved.clone());
            resolved
        } else {
            ccs_p.name.clone()
        };

        items.push(DetectItem {
            original_id: ccs_p.id.clone(),
            name: ccs_p.name.clone(),
            base_url,
            has_api_key: api_key.is_some(),
            model,
            website_url: ccs_p.website_url.clone(),
            importable,
            conflict,
            imported_name,
            already_imported,
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

// ── import ───────────────────────────────────────────────────────

/// 内联 API key 加密，与 `http/api/mod.rs::encrypt_api_key` 一致：
/// `crypto.encrypt(json!({"api_key": k}), endpoint_id.as_bytes())`。
///
/// service 层不复用 HTTP 层的 `encrypt_api_key`（其签名为 `&AppState` 且
/// `pub(crate)`），直接内联同样逻辑以解耦 state。
fn encrypt_api_key(
    crypto: Option<&CryptoService>,
    endpoint_id: &str,
    api_key: Option<&str>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(k) = api_key else {
        return Ok(None);
    };
    let crypto = crypto.ok_or_else(|| "系统凭据管理器不可用，无法保存凭据".to_string())?;
    let plaintext = serde_json::to_vec(&json!({ "api_key": k }))
        .map_err(|e| format!("序列化凭据失败: {}", e))?;
    let blob = crypto
        .encrypt(&plaintext, endpoint_id.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;
    Ok(Some(blob))
}

/// 批量导入：每个 item 重新读 ccs 源数据定位 → 建 endpoint + 建 provider；
/// 单项失败记入 errors，其余继续（非全有全无）。
///
/// `crypto=None` 且 item 含 api_key → 该项记入 errors 不创建 endpoint。
/// 重新读源数据（不信任前端传 base_url/token）：先探 SQLite，再探 config.json；
/// 两者都不存在 → Err。
pub fn import(
    db: &Mutex<Connection>,
    crypto: Option<&CryptoService>,
    config_path: Option<PathBuf>,
    sqlite_path: Option<PathBuf>,
    items: Vec<ImportItem>,
) -> Result<ImportResponse, String> {
    let (source_providers, source, _path) = read_ccs_providers(config_path, sqlite_path)?;
    if source == CcsSource::None {
        return Err("ccs 数据源不存在（无 cc-switch.db 且无 config.json），无法导入".to_string());
    }
    // 按 id 建索引，便于按 original_id 定位（Vec 已按 id 升序，无重复 id）。
    let by_id: HashMap<&str, &CcsSourceProvider> = source_providers
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();

    // 本地 name 集合 + 已导入 original_id 集合；导入过程中实时更新，
    // 避免批量内部同名互相撞名 + 二次导入幂等校验。
    let (mut local_names, mut imported_ids) = collect_local(db)?;

    for item in items {
        let ccs_p = match by_id.get(item.original_id.as_str()) {
            Some(p) => *p,
            None => {
                skipped.push(ImportSkip {
                    original_id: item.original_id,
                    reason: "ccs 中不存在该 id".to_string(),
                });
                continue;
            }
        };

        // 二次校验幂等：已导入过的不再创建。
        if imported_ids.contains(&ccs_p.id) {
            skipped.push(ImportSkip {
                original_id: ccs_p.id.clone(),
                reason: "已导入过".to_string(),
            });
            continue;
        }

        let (base_url, api_key, model) = extract_env(&ccs_p.settings_config);
        let Some(base_url) = base_url else {
            skipped.push(ImportSkip {
                original_id: ccs_p.id.clone(),
                reason: "无 base_url".to_string(),
            });
            continue;
        };

        // 确定最终名称：信任前端传入的 imported_name（含冲突后缀），但仍做一道
        // 本地查重——若与本地已有 name 撞名则重新计算，避免前端漏算或并发新建。
        let imported_name = if local_names.contains(&item.imported_name) {
            resolve_unique_name(&item.imported_name, &local_names)
        } else {
            item.imported_name.clone()
        };

        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let api_key_encrypted = match encrypt_api_key(crypto, &endpoint_id, api_key.as_deref()) {
            Ok(blob) => blob,
            Err(e) => {
                errors.push(ImportError {
                    original_id: ccs_p.id.clone(),
                    message: e,
                });
                continue;
            }
        };

        let new_ep = endpoints::NewEndpoint {
            id: endpoint_id.clone(),
            account_id: None,
            name: imported_name.clone(),
            base_url: base_url.clone(),
            protocol_type: "anthropic".to_string(),
            api_key_encrypted,
            auth_mode: "api_key".to_string(),
            priority: 0,
            extra_json: None,
        };
        let endpoint = match endpoints::create(db, new_ep) {
            Ok(e) => e,
            Err(e) => {
                errors.push(ImportError {
                    original_id: ccs_p.id.clone(),
                    message: format!("创建端点失败: {}", e),
                });
                continue;
            }
        };

        let provider_id = uuid::Uuid::new_v4().to_string();
        let mut settings = json!({ "endpoint_id": endpoint.id });
        if let Some(m) = &model {
            settings["model"] = json!(m);
        }
        let meta = json!({
            "imported_from": "ccs",
            "original_id": ccs_p.id,
            "website_url": ccs_p.website_url,
        });

        let sort_index = providers::next_sort_index(db, "claude-code")?;
        let new_prov = providers::NewProvider {
            id: provider_id.clone(),
            app_type: "claude-code".to_string(),
            name: imported_name.clone(),
            mode: "direct".to_string(),
            settings_config: settings.to_string(),
            category: Some("custom".to_string()),
            sort_index: Some(sort_index),
            notes: None,
            meta: meta.to_string(),
        };
        if let Err(e) = providers::create(db, new_prov) {
            // provider 失败 → 回滚 endpoint（delete 失败不掩盖原错误）。
            let rollback_msg = match endpoints::delete(db, &endpoint.id) {
                Ok(()) => String::new(),
                Err(re) => format!("（回滚端点失败: {}）", re),
            };
            errors.push(ImportError {
                original_id: ccs_p.id.clone(),
                message: format!("创建 provider 失败: {}{}", e, rollback_msg),
            });
            continue;
        }

        // 成功：登记 name 与 original_id，避免后续 item 撞名/重复导入。
        local_names.insert(imported_name.clone());
        imported_ids.insert(ccs_p.id.clone());

        created.push(ImportedProvider {
            original_id: ccs_p.id.clone(),
            provider_id,
            endpoint_id: endpoint.id,
            name: imported_name,
        });
    }

    Ok(ImportResponse {
        created_providers: created,
        skipped,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 唯一临时目录（每个测试独立，避免并发污染）。
    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("as-ccs-test-{}-{}-{}", tag, pid, n));
        // 清理可能残留的旧目录。
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn test_crypto() -> CryptoService {
        CryptoService::new(crate::services::crypto::generate_master_key())
    }

    /// 写 mock ccs config.json 到 dir，返回其路径。
    fn write_ccs_config(dir: &Path, json: &str) -> PathBuf {
        let path = dir.join("config.json");
        std::fs::write(&path, json).expect("写 config.json 失败");
        path
    }

    /// 构造一个 ccs provider JSON 片段（settingsConfig 即完整 settings.json）。
    fn ccs_provider(id: &str, name: &str, settings: Value, website: Option<&str>) -> Value {
        let mut v = json!({
            "id": id,
            "name": name,
            "settingsConfig": settings,
        });
        if let Some(w) = website {
            v["websiteUrl"] = json!(w);
        }
        v
    }

    /// mock ccs provider 的 SQLite 行输入。
    struct SqliteProviderRow<'a> {
        id: &'a str,
        name: &'a str,
        /// settings_config 存字符串（Claude Code settings.json 全文）。
        settings: Value,
        website_url: Option<&'a str>,
        category: Option<&'a str>,
    }

    /// 在 dir 建一个 mock ccs `cc-switch.db`（复刻新版 main 分支 schema），
    /// 插入若干 claude provider 行，返回其路径。
    ///
    /// 仅建 detect/import 所需的最小 schema（复合主键 (id, app_type)）；
    /// 额外插一行 codex provider 以验证 detect 只取 claude。
    fn write_ccs_sqlite(dir: &Path, rows: &[SqliteProviderRow<'_>]) -> PathBuf {
        let path = dir.join("cc-switch.db");
        let conn = Connection::open(&path).expect("创建 cc-switch.db 失败");
        conn.execute_batch(
            "CREATE TABLE providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                website_url TEXT,
                category TEXT,
                created_at INTEGER,
                sort_index INTEGER,
                notes TEXT,
                icon TEXT,
                icon_color TEXT,
                meta TEXT NOT NULL DEFAULT '{}',
                is_current BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            );",
        )
        .expect("建表失败");
        for r in rows {
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, website_url, category)
                 VALUES (?1, 'claude', ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    r.id,
                    r.name,
                    r.settings.to_string(),
                    r.website_url,
                    r.category,
                ],
            )
            .expect("插入 claude provider 失败");
        }
        // 插一行 codex provider，验证 detect/import 只取 app_type='claude'。
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config)
             VALUES ('codex-1', 'codex', 'Codex渠道', '{}')",
            [],
        )
        .expect("插入 codex provider 失败");
        path
    }

    fn sqlite_row<'a>(id: &'a str, name: &'a str, settings: Value) -> SqliteProviderRow<'a> {
        SqliteProviderRow {
            id,
            name,
            settings,
            website_url: None,
            category: None,
        }
    }

    // ── resolve_unique_name ──────────────────────────────────────

    #[test]
    fn resolve_unique_name_no_conflict() {
        let existing = HashSet::new();
        assert_eq!(resolve_unique_name("DeepSeek", &existing), "DeepSeek");
    }

    #[test]
    fn resolve_unique_name_first_suffix() {
        let mut existing = HashSet::new();
        existing.insert("DeepSeek".to_string());
        assert_eq!(resolve_unique_name("DeepSeek", &existing), "DeepSeek (ccs)");
    }

    #[test]
    fn resolve_unique_name_increments() {
        let mut existing = HashSet::new();
        existing.insert("DeepSeek".to_string());
        existing.insert("DeepSeek (ccs)".to_string());
        existing.insert("DeepSeek (ccs 2)".to_string());
        assert_eq!(
            resolve_unique_name("DeepSeek", &existing),
            "DeepSeek (ccs 3)"
        );
    }

    // ── detect ───────────────────────────────────────────────────

    #[test]
    fn detect_file_not_found() {
        let dir = unique_dir("detect-not-found");
        let db = setup_db();
        // 不写 config.json 也不写 cc-switch.db → found=false。
        let resp = detect(
            &db,
            Some(dir.join("config.json")),
            Some(dir.join("cc-switch.db")),
        )
        .unwrap();
        assert!(!resp.found);
        assert!(resp.providers.is_empty());
        assert_eq!(resp.source, "none");
    }

    #[test]
    fn detect_normal_provider() {
        let dir = unique_dir("detect-normal");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                        "ANTHROPIC_MODEL": "deepseek-chat"
                    }
                }), Some("https://deepseek.com")),
            },
            "current": "p1"
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();

        let resp = detect(&db, Some(path), Some(dir.join("cc-switch.db"))).unwrap();
        assert!(resp.found);
        assert_eq!(resp.source, "config.json");
        assert_eq!(resp.providers.len(), 1);
        let item = &resp.providers[0];
        assert_eq!(item.original_id, "p1");
        assert_eq!(item.name, "DeepSeek");
        assert_eq!(
            item.base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert!(item.has_api_key);
        assert_eq!(item.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(item.website_url.as_deref(), Some("https://deepseek.com"));
        assert!(item.importable);
        assert!(!item.conflict);
        assert!(!item.already_imported);
        assert_eq!(item.imported_name, "DeepSeek");
        assert!(item.warning.is_none());
    }

    #[test]
    fn detect_empty_env_not_importable() {
        let dir = unique_dir("detect-empty-env");
        let cfg = json!({
            "providers": {
                "official": ccs_provider("official", "Claude官方登录", json!({"env": {}}), None),
            },
            "current": "official"
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();

        let resp = detect(&db, Some(path), Some(dir.join("cc-switch.db"))).unwrap();
        let item = &resp.providers[0];
        assert!(!item.importable, "空 env 应不可导入");
        assert!(item.base_url.is_none());
        assert!(!item.has_api_key);
        assert!(item.warning.is_some());
    }

    #[test]
    fn detect_conflict_renames() {
        let dir = unique_dir("detect-conflict");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        // 本地先建一个同名 provider。
        providers::create(
            &db,
            providers::NewProvider {
                id: "local-1".to_string(),
                app_type: "claude-code".to_string(),
                name: "DeepSeek".to_string(),
                mode: "proxy".to_string(),
                settings_config: "{}".to_string(),
                category: None,
                sort_index: None,
                notes: None,
                meta: "{}".to_string(),
            },
        )
        .unwrap();

        let resp = detect(&db, Some(path), Some(dir.join("cc-switch.db"))).unwrap();
        let item = &resp.providers[0];
        assert!(item.conflict, "应识别出冲突");
        assert_eq!(item.imported_name, "DeepSeek (ccs)");
        assert!(item.importable);
    }

    #[test]
    fn detect_already_imported() {
        let dir = unique_dir("detect-already");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        // 本地已有一条 meta.original_id=p1 的已导入 provider（同名也算，验证
        // already_imported 优先于 conflict）。
        providers::create(
            &db,
            providers::NewProvider {
                id: "imported-1".to_string(),
                app_type: "claude-code".to_string(),
                name: "DeepSeek".to_string(),
                mode: "direct".to_string(),
                settings_config: r#"{"endpoint_id":"ep-x"}"#.to_string(),
                category: Some("custom".to_string()),
                sort_index: None,
                notes: None,
                meta: r#"{"imported_from":"ccs","original_id":"p1"}"#.to_string(),
            },
        )
        .unwrap();

        let resp = detect(&db, Some(path), Some(dir.join("cc-switch.db"))).unwrap();
        let item = &resp.providers[0];
        assert!(item.already_imported, "应识别出已导入");
        assert!(
            !item.conflict,
            "已导入项即使同名也不标 conflict（默认不勾选，无需重命名）"
        );
        assert_eq!(item.imported_name, "DeepSeek");
    }

    // ── import ───────────────────────────────────────────────────

    fn import_item(id: &str, name: &str) -> ImportItem {
        ImportItem {
            original_id: id.to_string(),
            imported_name: name.to_string(),
        }
    }

    #[test]
    fn import_normal_creates_endpoint_and_provider() {
        let dir = unique_dir("import-normal");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                        "ANTHROPIC_MODEL": "deepseek-chat"
                    }
                }), Some("https://deepseek.com")),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();

        assert_eq!(resp.created_providers.len(), 1);
        assert!(resp.skipped.is_empty());
        assert!(resp.errors.is_empty());

        let created = &resp.created_providers[0];
        let prov = providers::get(&db, &created.provider_id).unwrap().unwrap();
        assert_eq!(prov.app_type, "claude-code");
        assert_eq!(prov.mode, "direct");
        assert_eq!(prov.name, "DeepSeek");
        assert!(!prov.is_current, "导入不激活");
        assert_eq!(prov.category.as_deref(), Some("custom"));
        let sc: Value = serde_json::from_str(&prov.settings_config).unwrap();
        assert_eq!(
            sc["endpoint_id"].as_str(),
            Some(created.endpoint_id.as_str())
        );
        assert_eq!(sc["model"].as_str(), Some("deepseek-chat"));
        let meta: Value = serde_json::from_str(&prov.meta).unwrap();
        assert_eq!(meta["imported_from"].as_str(), Some("ccs"));
        assert_eq!(meta["original_id"].as_str(), Some("p1"));
        assert_eq!(meta["website_url"].as_str(), Some("https://deepseek.com"));

        let ep = endpoints::get(&db, &created.endpoint_id).unwrap().unwrap();
        assert_eq!(ep.base_url, "https://api.deepseek.com/anthropic");
        assert_eq!(ep.protocol_type, "anthropic");
        assert_eq!(ep.auth_mode, "api_key");
        assert!(ep.api_key_encrypted.is_some(), "应加密保存 api_key");

        // 加密内容可被同一 crypto 解密回原 key。
        let plain = crypto
            .decrypt(ep.api_key_encrypted.as_ref().unwrap(), ep.id.as_bytes())
            .unwrap();
        let v: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v["api_key"].as_str(), Some("sk-xxx"));
    }

    #[test]
    fn import_conflict_renames_and_preserves_local() {
        let dir = unique_dir("import-conflict");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();
        // 本地已有同名。
        providers::create(
            &db,
            providers::NewProvider {
                id: "local-1".to_string(),
                app_type: "claude-code".to_string(),
                name: "DeepSeek".to_string(),
                mode: "proxy".to_string(),
                settings_config: "{}".to_string(),
                category: None,
                sort_index: None,
                notes: None,
                meta: "{}".to_string(),
            },
        )
        .unwrap();

        let resp = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            // 前端按 detect 预览传 imported_name="DeepSeek (ccs)"。
            vec![import_item("p1", "DeepSeek (ccs)")],
        )
        .unwrap();

        assert_eq!(resp.created_providers.len(), 1);
        let created = &resp.created_providers[0];
        assert_eq!(created.name, "DeepSeek (ccs)");
        // 本地原项保留。
        assert!(providers::get(&db, "local-1").unwrap().is_some());
        assert_eq!(providers::list_by_app(&db, "claude-code").unwrap().len(), 2);
    }

    #[test]
    fn import_empty_env_skipped() {
        let dir = unique_dir("import-empty-env");
        let cfg = json!({
            "providers": {
                "official": ccs_provider("official", "Claude官方", json!({"env": {}}), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("official", "Claude官方")],
        )
        .unwrap();
        assert!(resp.created_providers.is_empty());
        assert_eq!(resp.skipped.len(), 1);
        assert_eq!(resp.skipped[0].reason, "无 base_url");
    }

    #[test]
    fn import_idempotent_second_time_skipped() {
        let dir = unique_dir("import-idempotent");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        let first = import(
            &db,
            Some(&crypto),
            Some(path.clone()),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();
        assert_eq!(first.created_providers.len(), 1);

        let second = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();
        assert!(second.created_providers.is_empty(), "二次导入不应重复创建");
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(second.skipped[0].reason, "已导入过");
        // DB 中只应有 1 行 provider。
        assert_eq!(providers::list_by_app(&db, "claude-code").unwrap().len(), 1);
    }

    #[test]
    fn import_crypto_unavailable_with_api_key_errors() {
        let dir = unique_dir("import-no-crypto");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();

        let resp = import(
            &db,
            None,
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();
        assert!(
            resp.created_providers.is_empty(),
            "crypto 不可用且含 key 不应创建"
        );
        assert_eq!(resp.errors.len(), 1);
        assert!(resp.errors[0].message.contains("系统凭据管理器不可用"));
        // 不应残留 endpoint。
        assert!(endpoints::list(&db).unwrap().is_empty());
    }

    #[test]
    fn import_crypto_unavailable_no_api_key_still_creates() {
        let dir = unique_dir("import-no-crypto-nokey");
        // base_url 存在但无 AUTH_TOKEN（少见但合法：端点免鉴权）。
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "Open", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://example.com/anthropic"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();

        let resp = import(
            &db,
            None,
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "Open")],
        )
        .unwrap();
        assert_eq!(
            resp.created_providers.len(),
            1,
            "无 api_key 时 crypto 不可用仍可导入"
        );
        let ep = endpoints::get(&db, &resp.created_providers[0].endpoint_id)
            .unwrap()
            .unwrap();
        assert!(ep.api_key_encrypted.is_none());
    }

    #[test]
    fn import_missing_original_id_skipped() {
        let dir = unique_dir("import-missing-id");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("ghost", "Ghost")],
        )
        .unwrap();
        assert!(resp.created_providers.is_empty());
        assert_eq!(resp.skipped.len(), 1);
        assert_eq!(resp.skipped[0].reason, "ccs 中不存在该 id");
    }

    #[test]
    fn import_batch_internal_name_collision_resolved() {
        let dir = unique_dir("import-batch-collision");
        // 两个 ccs provider 同名 "DeepSeek"，批量导入时第二个应自动加后缀，
        // 不互相覆盖也不撞 endpoint。
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://a.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-1"
                    }
                }), None),
                "p2": ccs_provider("p2", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://b.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-2"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek"), import_item("p2", "DeepSeek")],
        )
        .unwrap();
        assert_eq!(resp.created_providers.len(), 2);
        let names: Vec<&str> = resp
            .created_providers
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"DeepSeek"));
        assert!(names.contains(&"DeepSeek (ccs)"));
        // 两个 endpoint 应指向不同 base_url。
        let eps: Vec<String> = resp
            .created_providers
            .iter()
            .map(|c| {
                endpoints::get(&db, &c.endpoint_id)
                    .unwrap()
                    .unwrap()
                    .base_url
            })
            .collect();
        assert!(eps.contains(&"https://a.example.com/anthropic".to_string()));
        assert!(eps.contains(&"https://b.example.com/anthropic".to_string()));
    }

    #[test]
    fn import_does_not_activate_provider() {
        let dir = unique_dir("import-no-activate");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let path = write_ccs_config(&dir, &cfg.to_string());
        let db = setup_db();
        let crypto = test_crypto();

        import(
            &db,
            Some(&crypto),
            Some(path),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();
        assert!(
            providers::get_current(&db, "claude-code")
                .unwrap()
                .is_none(),
            "导入不应激活任何 provider"
        );
    }

    #[test]
    fn import_does_not_modify_ccs_config() {
        let dir = unique_dir("import-readonly");
        let cfg = json!({
            "providers": {
                "p1": ccs_provider("p1", "DeepSeek", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
                    }
                }), None),
            },
            "current": ""
        });
        let cfg_text = cfg.to_string();
        let path = write_ccs_config(&dir, &cfg_text);
        let db = setup_db();
        let crypto = test_crypto();

        import(
            &db,
            Some(&crypto),
            Some(path.clone()),
            Some(dir.join("cc-switch.db")),
            vec![import_item("p1", "DeepSeek")],
        )
        .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, cfg_text, "导入不得修改 ccs config.json");
    }

    // ── SQLite 数据源（新版 main 分支）────────────────────────────

    #[test]
    fn detect_sqlite_normal_provider() {
        let dir = unique_dir("detect-sqlite-normal");
        let db_path = write_ccs_sqlite(
            &dir,
            &[SqliteProviderRow {
                id: "sq1",
                name: "SqliteDeepSeek",
                settings: json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                        "ANTHROPIC_MODEL": "deepseek-chat"
                    }
                }),
                website_url: Some("https://deepseek.com"),
                category: Some("aggregator"),
            }],
        );
        let db = setup_db();

        // config_path 传一个不存在的路径，确保只命中 SQLite。
        let resp = detect(&db, Some(dir.join("config.json")), Some(db_path)).unwrap();
        assert!(resp.found);
        assert_eq!(resp.source, "sqlite");
        // 只应取 claude provider（codex 行被过滤）。
        assert_eq!(resp.providers.len(), 1);
        let item = &resp.providers[0];
        assert_eq!(item.original_id, "sq1");
        assert_eq!(item.name, "SqliteDeepSeek");
        assert_eq!(
            item.base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert!(item.has_api_key);
        assert_eq!(item.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(item.website_url.as_deref(), Some("https://deepseek.com"));
        assert!(item.importable);
    }

    #[test]
    fn detect_sqlite_empty_env_not_importable() {
        let dir = unique_dir("detect-sqlite-empty");
        let db_path = write_ccs_sqlite(
            &dir,
            &[sqlite_row(
                "sq-official",
                "Claude官方",
                json!({ "env": {} }),
            )],
        );
        let db = setup_db();

        let resp = detect(&db, Some(dir.join("config.json")), Some(db_path)).unwrap();
        assert_eq!(resp.source, "sqlite");
        let item = &resp.providers[0];
        assert!(!item.importable, "空 env 应不可导入");
        assert!(item.base_url.is_none());
        assert!(!item.has_api_key);
        assert!(item.warning.is_some());
    }

    #[test]
    fn detect_neither_source_found_is_false() {
        let dir = unique_dir("detect-neither");
        let db = setup_db();
        // config.json 与 cc-switch.db 均不存在。
        let resp = detect(
            &db,
            Some(dir.join("config.json")),
            Some(dir.join("cc-switch.db")),
        )
        .unwrap();
        assert!(!resp.found);
        assert_eq!(resp.source, "none");
        assert!(resp.providers.is_empty());
    }

    #[test]
    fn detect_sqlite_takes_priority_over_config_json() {
        let dir = unique_dir("detect-priority");
        // config.json 与 cc-switch.db 均存在且各含一个不同的 provider。
        let cfg = json!({
            "providers": {
                "cfg1": ccs_provider("cfg1", "FromConfigJson", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://config.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-cfg"
                    }
                }), None),
            },
            "current": ""
        });
        let cfg_path = write_ccs_config(&dir, &cfg.to_string());
        let db_path = write_ccs_sqlite(
            &dir,
            &[sqlite_row(
                "sq1",
                "FromSqlite",
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://sqlite.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-sql"
                    }
                }),
            )],
        );
        let db = setup_db();

        let resp = detect(&db, Some(cfg_path), Some(db_path)).unwrap();
        // 两者都在时应用 SQLite。
        assert_eq!(resp.source, "sqlite");
        assert_eq!(resp.providers.len(), 1);
        assert_eq!(resp.providers[0].name, "FromSqlite");
    }

    #[test]
    fn import_sqlite_normal_creates_endpoint_and_provider() {
        let dir = unique_dir("import-sqlite-normal");
        let db_path = write_ccs_sqlite(
            &dir,
            &[SqliteProviderRow {
                id: "sq1",
                name: "SqliteDeepSeek",
                settings: json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                        "ANTHROPIC_MODEL": "deepseek-chat"
                    }
                }),
                website_url: Some("https://deepseek.com"),
                category: Some("aggregator"),
            }],
        );
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(dir.join("config.json")),
            Some(db_path),
            vec![import_item("sq1", "SqliteDeepSeek")],
        )
        .unwrap();

        assert_eq!(resp.created_providers.len(), 1);
        assert!(resp.skipped.is_empty());
        assert!(resp.errors.is_empty());

        let created = &resp.created_providers[0];
        let prov = providers::get(&db, &created.provider_id).unwrap().unwrap();
        assert_eq!(prov.app_type, "claude-code");
        assert_eq!(prov.mode, "direct");
        assert_eq!(prov.name, "SqliteDeepSeek");
        let meta: Value = serde_json::from_str(&prov.meta).unwrap();
        assert_eq!(meta["imported_from"].as_str(), Some("ccs"));
        assert_eq!(meta["original_id"].as_str(), Some("sq1"));

        let ep = endpoints::get(&db, &created.endpoint_id).unwrap().unwrap();
        assert_eq!(ep.base_url, "https://api.deepseek.com/anthropic");
        assert!(ep.api_key_encrypted.is_some(), "应加密保存 api_key");
        // 加密内容可被同一 crypto 解密回原 key（验证密钥落库加密而非明文）。
        let plain = crypto
            .decrypt(ep.api_key_encrypted.as_ref().unwrap(), ep.id.as_bytes())
            .unwrap();
        let v: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v["api_key"].as_str(), Some("sk-xxx"));
    }

    #[test]
    fn import_sqlite_empty_env_skipped() {
        let dir = unique_dir("import-sqlite-empty");
        let db_path = write_ccs_sqlite(
            &dir,
            &[sqlite_row(
                "sq-official",
                "Claude官方",
                json!({ "env": {} }),
            )],
        );
        let db = setup_db();
        let crypto = test_crypto();

        let resp = import(
            &db,
            Some(&crypto),
            Some(dir.join("config.json")),
            Some(db_path),
            vec![import_item("sq-official", "Claude官方")],
        )
        .unwrap();
        assert!(resp.created_providers.is_empty());
        assert_eq!(resp.skipped.len(), 1);
        assert_eq!(resp.skipped[0].reason, "无 base_url");
    }

    #[test]
    fn import_sqlite_takes_priority_over_config_json() {
        let dir = unique_dir("import-priority");
        // config.json 与 cc-switch.db 各含一个 provider，import 应只从 SQLite 取。
        let cfg = json!({
            "providers": {
                "sq1": ccs_provider("sq1", "FromConfigJson", json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://config.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-cfg"
                    }
                }), None),
            },
            "current": ""
        });
        let cfg_path = write_ccs_config(&dir, &cfg.to_string());
        let db_path = write_ccs_sqlite(
            &dir,
            &[sqlite_row(
                "sq1",
                "FromSqlite",
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://sqlite.example.com/anthropic",
                        "ANTHROPIC_AUTH_TOKEN": "sk-sql"
                    }
                }),
            )],
        );
        let db = setup_db();
        let crypto = test_crypto();

        // 同一 original_id "sq1" 在两个源里 base_url 不同；SQLite 优先。
        let resp = import(
            &db,
            Some(&crypto),
            Some(cfg_path),
            Some(db_path),
            vec![import_item("sq1", "FromSqlite")],
        )
        .unwrap();
        assert_eq!(resp.created_providers.len(), 1);
        let ep = endpoints::get(&db, &resp.created_providers[0].endpoint_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            ep.base_url, "https://sqlite.example.com/anthropic",
            "应从 SQLite 源取 base_url（优先于 config.json）"
        );
    }

    #[test]
    fn import_neither_source_errors() {
        let dir = unique_dir("import-neither");
        let db = setup_db();
        let crypto = test_crypto();

        let err = import(
            &db,
            Some(&crypto),
            Some(dir.join("config.json")),
            Some(dir.join("cc-switch.db")),
            vec![import_item("sq1", "X")],
        )
        .unwrap_err();
        assert!(err.contains("ccs 数据源不存在"));
    }
}
