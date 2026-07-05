pub mod claude_code;
pub mod codex;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::db::dao::endpoints as endpoints_dao;
use crate::db::dao::now_iso;
use crate::db::dao::providers::ProviderRow;
use crate::db::dao::tool_takeover as dao;
use crate::services::crypto::CryptoService;

// ── Shared constants ────────────────────────────────────────────────────────

/// 本地服务 base URL。
pub const LOCAL_BASE: &str = "http://127.0.0.1:42567";
/// Claude Code 本地路由段。
pub const CLAUDE_CODE_SUFFIX: &str = "/claude-code";
/// Codex 本地路由段。
pub const CODEX_SUFFIX: &str = "/codex";
/// 写入工具配置的鉴权占位符,绝不包含真实凭据。
pub const PLACEHOLDER_TOKEN: &str = "agent-switch-managed";
/// Codex model_provider 表名兼 display name。
pub const CODEX_PROVIDER_ID: &str = "agent-switch";

// ── Tool enum ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tool {
    ClaudeCode,
    Codex,
    OpenCode,
}

impl Tool {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude-code",
            Tool::Codex => "codex",
            Tool::OpenCode => "opencode",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "claude-code" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            "opencode" => Some(Tool::OpenCode),
            _ => None,
        }
    }

    pub fn supports_takeover(&self) -> bool {
        matches!(self, Tool::ClaudeCode | Tool::Codex)
    }

    pub fn config_dir(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        match self {
            Tool::ClaudeCode => Some(home.join(".claude")),
            Tool::Codex => Some(home.join(".codex")),
            Tool::OpenCode => None,
        }
    }
}

// ── Data types ──────────────────────────────────────────────────────────────

/// 当前指向类别(R5)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum LiveCategory {
    #[serde(rename = "agent_switch")]
    AgentSwitch,
    #[serde(rename = "official")]
    Official,
    #[serde(rename = "third_party")]
    ThirdParty,
    #[serde(rename = "unconfigured")]
    Unconfigured,
    #[serde(rename = "unrecognized")]
    Unrecognized,
}

/// 工具状态响应体。
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub tool: String,
    pub supports_takeover: bool,
    pub enabled: bool,
    pub live_category: LiveCategory,
    pub last_applied_at: Option<String>,
    pub last_target: Option<String>,
    pub last_error: Option<String>,
}

/// 备份记录响应体。
#[derive(Debug, Clone, Serialize)]
pub struct ToolBackupInfo {
    pub id: String,
    pub original_path: String,
    pub backup_path: String,
    pub original_existed: bool,
    pub takeover_target: Option<String>,
    pub created_at: String,
}

/// direct 模式写入工具配置所需的解析后数据（凭据已解密）。
///
/// 由 `resolve_direct_config` 从 provider.settings_config + 加密端点解密得到，
/// 传给各工具的 `apply_direct` 做纯文件写入。凭据只短暂存在于内存。
pub struct DirectConfig {
    /// provider id，用作 Codex `model_provider` 表名。
    pub provider_id: String,
    pub base_url: String,
    /// 解密后的真实 API key（明文，仅用于写入工具文件，不落 DB、不记日志）。
    pub api_key: String,
    pub model: Option<String>,
    /// Codex 专属：`wire_api`（缺省 "responses"）。
    pub wire_api: Option<String>,
    /// Codex 专属：`requires_openai_auth`（缺省 true）。
    pub requires_openai_auth: Option<bool>,
}

/// direct 模式 provider 的 settings_config JSON 结构。
///
/// 引用 endpoint_id 而非内联明文 key——凭据仍由 endpoints 表 AES-256-GCM 加密存储
/// （偏离 ccs 明文做法，见任务 PRD 决策）。
#[derive(Debug, Deserialize)]
struct DirectSettings {
    endpoint_id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    wire_api: Option<String>,
    #[serde(default)]
    requires_openai_auth: Option<bool>,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// 启用接管——备份 + 写入工具配置 + 持久化成功状态。
///
/// 薄包装：解析工具配置目录后委派给 `enable_at`（后者接收显式 config_dir 便于测试）。
pub fn enable(db: &Mutex<rusqlite::Connection>, tool: Tool, data_dir: &Path) -> Result<(), String> {
    let config_dir = resolve_config_dir(tool)?;
    enable_at(db, tool, &config_dir, data_dir)
}

/// `enable` 的核心（接收显式 config_dir，便于用临时目录测试）。
fn enable_at(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    config_dir: &Path,
    data_dir: &Path,
) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }

    // 1. 备份原配置(R3,R3.4)
    backup_before_write(db, tool, config_dir, data_dir)?;

    // 2. 写入工具配置，指向 agent-switch
    match tool {
        Tool::ClaudeCode => claude_code::apply(config_dir)?,
        Tool::Codex => codex::apply(config_dir)?,
        _ => unreachable!(),
    }

    // 3. 持久化成功状态
    let target = match tool {
        Tool::ClaudeCode => format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX),
        Tool::Codex => format!("{}{}", LOCAL_BASE, CODEX_SUFFIX),
        _ => unreachable!(),
    };
    let now = now_iso()?;
    dao::upsert_state(
        db,
        tool.as_str(),
        true,
        "proxy",
        None,
        Some(&now),
        Some(&target),
        None,
    )?;

    Ok(())
}

/// 以 direct（直连）模式启用接管。
///
/// 薄包装：解析工具配置目录后委派给 `enable_direct_at`（接收显式 config_dir 便于测试）。
pub fn enable_direct(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    data_dir: &Path,
    provider: &ProviderRow,
    crypto: Option<&CryptoService>,
) -> Result<(), String> {
    let config_dir = resolve_config_dir(tool)?;
    enable_direct_at(db, tool, &config_dir, data_dir, provider, crypto)
}

/// `enable_direct` 的核心（接收显式 config_dir，便于用临时目录测试）。
fn enable_direct_at(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    config_dir: &Path,
    data_dir: &Path,
    provider: &ProviderRow,
    crypto: Option<&CryptoService>,
) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }
    if provider.app_type != tool.as_str() {
        return Err(format!(
            "provider '{}' 的 app_type '{}' 与工具 '{}' 不匹配",
            provider.id,
            provider.app_type,
            tool.as_str()
        ));
    }
    if provider.mode != "direct" {
        return Err(format!("provider '{}' 不是 direct 模式", provider.id));
    }

    // 解析 + 解密（写文件前完成；失败则不动工具文件，避免部分写入）
    let cfg = resolve_direct_config(db, provider, crypto)?;

    // 备份原配置
    backup_before_write(db, tool, config_dir, data_dir)?;

    // 写真实配置
    match tool {
        Tool::ClaudeCode => claude_code::apply_direct(config_dir, &cfg)?,
        Tool::Codex => codex::apply_direct(config_dir, &cfg)?,
        _ => unreachable!(),
    }

    // 持久化状态：mode=direct、active_provider_id=provider.id、target=真实 base_url。
    let now = now_iso()?;
    dao::upsert_state(
        db,
        tool.as_str(),
        true,
        "direct",
        Some(&provider.id),
        Some(&now),
        Some(&cfg.base_url),
        None,
    )?;

    Ok(())
}

/// 从 provider.settings_config + 加密端点解析出 direct 写入所需数据（含解密）。
///
/// direct provider 的 settings_config 引用 endpoint_id，而非内联明文 key；
/// 凭据仍由 endpoints 表 AES-256-GCM 加密（AAD=endpoint.id），此处解密路径
/// 与 `auth_injector` 一致。
fn resolve_direct_config(
    db: &Mutex<rusqlite::Connection>,
    provider: &ProviderRow,
    crypto: Option<&CryptoService>,
) -> Result<DirectConfig, String> {
    let settings: DirectSettings = serde_json::from_str(&provider.settings_config)
        .map_err(|e| format!("解析 provider settings_config 失败: {}", e))?;

    let endpoint = endpoints_dao::get(db, &settings.endpoint_id)
        .map_err(|e| format!("查询端点失败: {}", e))?
        .ok_or_else(|| {
            format!(
                "direct provider 引用的端点 '{}' 不存在",
                settings.endpoint_id
            )
        })?;

    let crypto = crypto.ok_or_else(|| "加密服务不可用，无法解密 direct 凭据".to_string())?;
    let encrypted = endpoint
        .api_key_encrypted
        .as_ref()
        .ok_or_else(|| format!("端点 '{}' 缺少 api_key_encrypted", endpoint.name))?;
    let plaintext = crypto
        .decrypt(encrypted, endpoint.id.as_bytes())
        .map_err(|e| format!("解密 API Key 失败: {}", e))?;
    let json: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|e| format!("解析 API Key JSON 失败: {}", e))?;
    let api_key = json
        .get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "API Key 格式无效：缺少 api_key 字段".to_string())?
        .to_string();

    Ok(DirectConfig {
        provider_id: provider.id.clone(),
        base_url: endpoint.base_url.clone(),
        api_key,
        model: settings.model,
        wire_api: settings.wire_api,
        requires_openai_auth: settings.requires_openai_auth,
    })
}

/// 关闭接管的语义随当前模式而定：
/// - direct 模式：回退到 proxy 接管（重写为本地代理配置，清除真实凭据），
///   `mode='proxy'`、`active_provider_id=NULL`、`enabled` 保持 1。不让用户裸奔。
/// - proxy 模式：`enabled=0`，不改写工具文件（现有 R7 行为）。
pub fn disable(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    data_dir: &Path,
) -> Result<(), String> {
    let config_dir = resolve_config_dir(tool)?;
    disable_at(db, tool, &config_dir, data_dir)
}

/// `disable` 的核心（接收显式 config_dir，便于用临时目录测试）。
fn disable_at(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    config_dir: &Path,
    data_dir: &Path,
) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }
    let state =
        dao::get_state(db, tool.as_str()).map_err(|e| format!("查询接管状态失败: {}", e))?;
    let mode = state.as_ref().map(|s| s.mode.as_str()).unwrap_or("proxy");

    if mode == "direct" {
        // direct → 回退 proxy：重写工具配置为本地代理，清除真实凭据。
        backup_before_write(db, tool, config_dir, data_dir)?;
        match tool {
            Tool::ClaudeCode => claude_code::apply(config_dir)?,
            Tool::Codex => codex::apply(config_dir)?,
            _ => unreachable!(),
        }
        let target = match tool {
            Tool::ClaudeCode => format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX),
            Tool::Codex => format!("{}{}", LOCAL_BASE, CODEX_SUFFIX),
            _ => unreachable!(),
        };
        let now = now_iso()?;
        dao::upsert_state(
            db,
            tool.as_str(),
            true,
            "proxy",
            None,
            Some(&now),
            Some(&target),
            None,
        )?;
    } else {
        dao::set_enabled(db, tool.as_str(), false)?;
    }
    Ok(())
}

/// 重新应用接管(R4.2，幂等)。要求已开启接管。
///
/// mode-aware：direct 模式重新应用直连配置（需 crypto + 激活 provider）；
/// proxy 模式重新应用代理配置。direct 但缺激活 provider 时报错，绝不静默降级为 proxy。
pub fn reapply(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    data_dir: &Path,
    crypto: Option<&CryptoService>,
) -> Result<(), String> {
    let config_dir = resolve_config_dir(tool)?;
    reapply_at(db, tool, &config_dir, data_dir, crypto)
}

/// `reapply` 的核心（接收显式 config_dir，便于用临时目录测试）。
fn reapply_at(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    config_dir: &Path,
    data_dir: &Path,
    crypto: Option<&CryptoService>,
) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }
    let state = dao::get_state(db, tool.as_str())
        .map_err(|e| format!("查询接管状态失败: {}", e))?
        .ok_or_else(|| format!("工具 '{}' 未开启接管，无法重新应用", tool.as_str()))?;
    if !state.enabled {
        return Err(format!("工具 '{}' 未开启接管，无法重新应用", tool.as_str()));
    }

    if state.mode == "direct" {
        let provider_id = state.active_provider_id.ok_or_else(|| {
            format!(
                "工具 '{}' 处于 direct 模式但缺少激活 provider，无法重新应用",
                tool.as_str()
            )
        })?;
        let provider = crate::db::dao::providers::get(db, &provider_id)
            .map_err(|e| format!("查询 provider 失败: {}", e))?
            .ok_or_else(|| {
                format!(
                    "direct 模式激活的 provider '{}' 不存在，无法重新应用",
                    provider_id
                )
            })?;
        enable_direct_at(db, tool, config_dir, data_dir, &provider, crypto)
    } else {
        enable_at(db, tool, config_dir, data_dir)
    }
}

/// 获取工具状态（含实时 detect 检测）。
pub fn status(db: &Mutex<rusqlite::Connection>, tool: Tool) -> Result<ToolStatus, String> {
    let state =
        dao::get_state(db, tool.as_str()).map_err(|e| format!("查询接管状态失败: {}", e))?;

    let live_category = if tool.supports_takeover() {
        tool.config_dir()
            .map(|d| match tool {
                Tool::ClaudeCode => claude_code::detect(&d),
                Tool::Codex => codex::detect(&d),
                _ => LiveCategory::Unconfigured,
            })
            .unwrap_or(LiveCategory::Unconfigured)
    } else {
        LiveCategory::Unconfigured
    };

    Ok(ToolStatus {
        tool: tool.as_str().to_string(),
        supports_takeover: tool.supports_takeover(),
        enabled: state.as_ref().map(|s| s.enabled).unwrap_or(false),
        live_category,
        last_applied_at: state.as_ref().and_then(|s| s.last_applied_at.clone()),
        last_target: state.as_ref().and_then(|s| s.last_target.clone()),
        last_error: state.as_ref().and_then(|s| s.last_error.clone()),
    })
}

/// 列出所有工具的实时状态。
pub fn list_statuses(db: &Mutex<rusqlite::Connection>) -> Result<Vec<ToolStatus>, String> {
    let all_tools = [Tool::ClaudeCode, Tool::Codex, Tool::OpenCode];
    let mut out = Vec::with_capacity(all_tools.len());
    for t in &all_tools {
        out.push(status(db, *t)?);
    }
    Ok(out)
}

/// 列出指定工具的备份记录。
pub fn list_backups(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
) -> Result<Vec<ToolBackupInfo>, String> {
    let rows = dao::list_backups(db, tool.as_str())?;
    Ok(rows
        .into_iter()
        .map(|r| ToolBackupInfo {
            id: r.id,
            original_path: r.original_path,
            backup_path: r.backup_path,
            original_existed: r.original_existed,
            takeover_target: r.takeover_target,
            created_at: r.created_at,
        })
        .collect())
}

// ── Backup ──────────────────────────────────────────────────────────────────

/// 写前备份(R3)。
///
/// 若配置已是 agent-switch 接管态，跳过备份(R3.4)，不产生新备份记录。
/// 否则复制原文件到备份目录；原文件不存在时仅记录 original_existed=0。
fn backup_before_write(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    config_dir: &Path,
    data_dir: &Path,
) -> Result<(), String> {
    let current = match tool {
        Tool::ClaudeCode => claude_code::detect(config_dir),
        Tool::Codex => codex::detect(config_dir),
        _ => return Ok(()),
    };
    if current == LiveCategory::AgentSwitch {
        return Ok(());
    }

    let backup_root = data_dir.join("backups").join("tools");
    std::fs::create_dir_all(&backup_root)
        .map_err(|_| format!("创建备份目录失败: {}", backup_root.display()))?;

    let original_path = match tool {
        Tool::ClaudeCode => config_dir.join("settings.json"),
        Tool::Codex => config_dir.join("config.toml"),
        _ => return Ok(()),
    };

    let timestamp = now_iso()?;
    // Windows 文件名不允许 ':', 替换为 '-'
    let safe_ts = timestamp.replace(':', "-");
    let safe_name = original_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let backup_filename = format!("{}-{}-{}.bak", tool.as_str(), safe_name, safe_ts);
    let backup_path = backup_root.join(&backup_filename);

    let original_existed = if original_path.exists() {
        std::fs::copy(&original_path, &backup_path).map_err(|e| format!("备份失败: {}", e))?;
        true
    } else {
        false
    };

    let takeover_target = match tool {
        Tool::ClaudeCode => Some(format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX)),
        Tool::Codex => Some(format!("{}{}", LOCAL_BASE, CODEX_SUFFIX)),
        _ => None,
    };

    let id = uuid::Uuid::new_v4().to_string();
    dao::insert_backup(
        db,
        &id,
        tool.as_str(),
        &original_path.to_string_lossy(),
        &backup_path.to_string_lossy(),
        original_existed,
        takeover_target.as_deref(),
    )
    .map_err(|e| format!("写入备份记录失败: {}", e))?;

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// 解析工具配置目录（`~/.claude` / `~/.codex`）。主目录不可用时报错。
fn resolve_config_dir(tool: Tool) -> Result<PathBuf, String> {
    tool.config_dir()
        .ok_or_else(|| "无法获取用户主目录".to_string())
}

/// 原子写：写到 `.tmp` 再 rename 覆盖，避免写一半损坏用户配置。
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {}", parent.display(), e))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)
        .map_err(|e| format!("写入临时文件 {} 失败: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("重命名 {} -> {} 失败: {}", tmp.display(), path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::dao::endpoints::{self, NewEndpoint};
    use crate::db::dao::providers::{self, NewProvider};
    use crate::db::migrations::run_migrations;
    use crate::services::crypto::{generate_master_key, CryptoService};
    use rusqlite::Connection;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 每个测试用独立临时目录，避免污染真实 `~/.claude` / `~/.codex` 并防并发冲突。
    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("as-takeover-test-{}-{}-{}", tag, pid, n));
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
        CryptoService::new(generate_master_key())
    }

    /// 插入一条带加密 api_key 的端点，返回 endpoint id。
    fn insert_endpoint(
        db: &Mutex<Connection>,
        crypto: &CryptoService,
        id: &str,
        base_url: &str,
        protocol_type: &str,
        api_key: &str,
    ) {
        let plaintext = serde_json::to_vec(&json!({ "api_key": api_key })).unwrap();
        let encrypted = crypto.encrypt(&plaintext, id.as_bytes()).unwrap();
        endpoints::create(
            db,
            NewEndpoint {
                id: id.to_string(),
                account_id: None,
                name: format!("ep-{}", id),
                base_url: base_url.to_string(),
                protocol_type: protocol_type.to_string(),
                api_key_encrypted: Some(encrypted),
                auth_mode: "apikey".to_string(),
                priority: 0,
                extra_json: None,
            },
        )
        .unwrap();
    }

    /// 插入一条 direct 模式 provider（settings_config 引用 endpoint_id）。
    fn insert_direct_provider(
        db: &Mutex<Connection>,
        id: &str,
        app_type: &str,
        settings: Value,
    ) -> providers::ProviderRow {
        providers::create(
            db,
            NewProvider {
                id: id.to_string(),
                app_type: app_type.to_string(),
                name: format!("prov-{}", id),
                mode: "direct".to_string(),
                settings_config: settings.to_string(),
                category: Some("custom".to_string()),
                sort_index: None,
                notes: None,
                meta: "{}".to_string(),
            },
        )
        .unwrap()
    }

    // ── proxy 模式产物：占位符 + 本地代理 URL，绝无真实 key ──────────────────

    #[test]
    fn enable_proxy_claude_writes_placeholder_not_real_key() {
        let db = setup_db();
        let dir = unique_dir("proxy-claude");
        enable_at(&db, Tool::ClaudeCode, &dir, &dir).unwrap();

        let content = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        let root: Value = serde_json::from_str(&content).unwrap();
        let env = &root["env"];
        assert_eq!(
            env["ANTHROPIC_BASE_URL"].as_str().unwrap(),
            format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX)
        );
        assert_eq!(
            env["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap(),
            PLACEHOLDER_TOKEN
        );
        // proxy 态状态记录：mode=proxy、无激活 provider
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "proxy");
        assert!(state.active_provider_id.is_none());
        assert!(state.enabled);
    }

    #[test]
    fn enable_proxy_codex_writes_placeholder_not_real_key() {
        let db = setup_db();
        let dir = unique_dir("proxy-codex");
        enable_at(&db, Tool::Codex, &dir, &dir).unwrap();

        let toml = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(toml.contains(&format!("model_provider = \"{}\"", CODEX_PROVIDER_ID)));
        assert!(toml.contains(&format!("{}{}", LOCAL_BASE, CODEX_SUFFIX)));

        let auth = std::fs::read_to_string(dir.join("auth.json")).unwrap();
        let auth_json: Value = serde_json::from_str(&auth).unwrap();
        assert_eq!(
            auth_json["OPENAI_API_KEY"].as_str().unwrap(),
            PLACEHOLDER_TOKEN
        );
    }

    // ── direct 模式产物：真实 base_url + 解密后的真实 key ────────────────────

    #[test]
    fn enable_direct_claude_writes_real_base_url_and_key() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("direct-claude");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://real.example.com",
            "anthropic",
            "sk-real-secret",
        );
        let provider = insert_direct_provider(
            &db,
            "p1",
            "claude-code",
            json!({ "endpoint_id": "ep1", "model": "claude-sonnet-4-6" }),
        );

        enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, Some(&crypto)).unwrap();

        let content = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        let root: Value = serde_json::from_str(&content).unwrap();
        let env = &root["env"];
        assert_eq!(
            env["ANTHROPIC_BASE_URL"].as_str().unwrap(),
            "https://real.example.com"
        );
        assert_eq!(
            env["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap(),
            "sk-real-secret"
        );
        assert_eq!(
            env["ANTHROPIC_MODEL"].as_str().unwrap(),
            "claude-sonnet-4-6"
        );

        // 状态记录：mode=direct、active_provider_id=p1、target=真实 base_url
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "direct");
        assert_eq!(state.active_provider_id.as_deref(), Some("p1"));
        assert_eq!(
            state.last_target.as_deref(),
            Some("https://real.example.com")
        );
    }

    #[test]
    fn enable_direct_codex_writes_real_key_to_auth() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("direct-codex");
        insert_endpoint(
            &db,
            &crypto,
            "epc",
            "https://codex.example.com/v1",
            "openai-responses",
            "sk-codex-real",
        );
        let provider = insert_direct_provider(
            &db,
            "pc",
            "codex",
            json!({ "endpoint_id": "epc", "wire_api": "responses", "requires_openai_auth": true }),
        );

        enable_direct_at(&db, Tool::Codex, &dir, &dir, &provider, Some(&crypto)).unwrap();

        let auth = std::fs::read_to_string(dir.join("auth.json")).unwrap();
        let auth_json: Value = serde_json::from_str(&auth).unwrap();
        assert_eq!(
            auth_json["OPENAI_API_KEY"].as_str().unwrap(),
            "sk-codex-real"
        );

        let toml = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(toml.contains("https://codex.example.com/v1"));
        // model_provider 应指向 provider id（pc），而非固定 agent-switch
        assert!(toml.contains("model_provider = \"pc\""));
    }

    // ── direct → disable 回退 proxy：清除真实凭据 ───────────────────────────

    #[test]
    fn disable_direct_falls_back_to_proxy_and_clears_real_key() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("disable-direct");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://real.example.com",
            "anthropic",
            "sk-real",
        );
        let provider =
            insert_direct_provider(&db, "p1", "claude-code", json!({ "endpoint_id": "ep1" }));
        enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, Some(&crypto)).unwrap();

        // disable：direct → 回退 proxy
        disable_at(&db, Tool::ClaudeCode, &dir, &dir).unwrap();

        let content = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        let root: Value = serde_json::from_str(&content).unwrap();
        let env = &root["env"];
        // 真实凭据被清除，改回占位符 + 本地代理
        assert_eq!(
            env["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap(),
            PLACEHOLDER_TOKEN
        );
        assert_eq!(
            env["ANTHROPIC_BASE_URL"].as_str().unwrap(),
            format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX)
        );
        assert!(
            !content.contains("sk-real"),
            "真实 key 不应残留在配置文件中"
        );

        // 状态：mode 回 proxy、active_provider_id 清空、enabled 保持 1
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "proxy");
        assert!(state.active_provider_id.is_none());
        assert!(state.enabled, "回退 proxy 后仍应保持接管开启");
    }

    #[test]
    fn disable_proxy_only_sets_enabled_false() {
        let db = setup_db();
        let dir = unique_dir("disable-proxy");
        enable_at(&db, Tool::ClaudeCode, &dir, &dir).unwrap();
        let before = std::fs::read_to_string(dir.join("settings.json")).unwrap();

        disable_at(&db, Tool::ClaudeCode, &dir, &dir).unwrap();

        // proxy disable 不改写工具文件（R7）
        let after = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert_eq!(before, after, "proxy 态 disable 不应改写工具文件");
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert!(!state.enabled);
        assert_eq!(state.mode, "proxy");
    }

    // ── reapply mode-aware：direct 态不降级为 proxy ─────────────────────────

    #[test]
    fn reapply_direct_reapplies_direct_not_proxy() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("reapply-direct");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://real.example.com",
            "anthropic",
            "sk-real",
        );
        let provider =
            insert_direct_provider(&db, "p1", "claude-code", json!({ "endpoint_id": "ep1" }));
        enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, Some(&crypto)).unwrap();

        // 外部篡改文件后 reapply，应恢复 direct 配置（真实 key），而非降级 proxy
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        reapply_at(&db, Tool::ClaudeCode, &dir, &dir, Some(&crypto)).unwrap();

        let content = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(
            content.contains("sk-real"),
            "reapply 应恢复 direct 真实凭据"
        );
        assert!(content.contains("https://real.example.com"));
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "direct", "reapply 不应把 direct 降级为 proxy");
    }

    #[test]
    fn reapply_direct_missing_provider_errors_not_downgrade() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("reapply-missing");
        // 手工构造 direct 态但 active_provider_id 指向不存在的 provider
        dao::upsert_state(
            &db,
            "claude-code",
            true,
            "direct",
            Some("ghost"),
            None,
            None,
            None,
        )
        .unwrap();

        let err = reapply_at(&db, Tool::ClaudeCode, &dir, &dir, Some(&crypto)).unwrap_err();
        assert!(err.contains("不存在"), "缺失 provider 应报错: {}", err);
        // 状态未被静默改成 proxy
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "direct", "报错时不应降级为 proxy");
    }

    // ── apply_direct 端点缺失/无凭据：报错且不部分写文件 ────────────────────

    #[test]
    fn enable_direct_missing_endpoint_errors_without_writing() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("direct-noep");
        let provider = insert_direct_provider(
            &db,
            "p1",
            "claude-code",
            json!({ "endpoint_id": "does-not-exist" }),
        );

        let err = enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, Some(&crypto))
            .unwrap_err();
        assert!(err.contains("不存在"), "应报端点不存在: {}", err);
        // 不应写出工具文件
        assert!(
            !dir.join("settings.json").exists(),
            "解析失败时不应部分写入工具文件"
        );
    }

    #[test]
    fn enable_direct_without_crypto_errors() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("direct-nocrypto");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://x.example.com",
            "anthropic",
            "sk-x",
        );
        let provider =
            insert_direct_provider(&db, "p1", "claude-code", json!({ "endpoint_id": "ep1" }));

        let err = enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, None).unwrap_err();
        assert!(err.contains("加密服务不可用"), "无 crypto 应报错: {}", err);
    }

    #[test]
    fn enable_direct_rejects_app_type_mismatch() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("direct-mismatch");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://x.example.com",
            "anthropic",
            "sk-x",
        );
        // provider app_type=codex，却用 ClaudeCode 工具启用
        let provider = insert_direct_provider(&db, "p1", "codex", json!({ "endpoint_id": "ep1" }));

        let err = enable_direct_at(&db, Tool::ClaudeCode, &dir, &dir, &provider, Some(&crypto))
            .unwrap_err();
        assert!(err.contains("不匹配"), "app_type 不匹配应报错: {}", err);
    }
}
