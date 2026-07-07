pub mod claude_code;
pub mod claude_snapshot;
pub mod codex;
pub mod json_merge;

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

/// Claude Code common config 默认值（对齐 ccs：提交时不追加 Co-Authored-By）。
pub const COMMON_CONFIG_CLAUDE_DEFAULT: &str = r#"{"includeCoAuthoredBy":false}"#;
/// common config 三态开关未显式设置时的默认行为（默认启用，对齐 ccs）。
const COMMON_CONFIG_DEFAULT_ENABLED: bool = true;

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

// ── Common config（跨 provider 全局层）─────────────────────────────────────────

/// 读取 Claude Code 的 common config 片段（`app_metadata` 键 `common_config_claude-code`）。
///
/// 未设置 → 返回 `None`（不叠加）。存储值为空串或非法 JSON → 视为未设置。
pub fn read_common_config(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
) -> Result<Option<serde_json::Value>, String> {
    let key = common_config_key(tool);
    let raw = crate::db::dao::app_metadata::get(db, &key)?;
    Ok(raw
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.is_object()))
}

/// 写入 Claude Code 的 common config 片段（裸 JSON，须为对象）。
pub fn write_common_config(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    value: &serde_json::Value,
) -> Result<(), String> {
    if !value.is_object() {
        return Err("common config 必须是 JSON 对象".to_string());
    }
    let key = common_config_key(tool);
    let text =
        serde_json::to_string(value).map_err(|e| format!("序列化 common config 失败: {}", e))?;
    crate::db::dao::app_metadata::set(db, &key, &text)
}

/// common config 在 `app_metadata` 中的键名。
fn common_config_key(tool: Tool) -> String {
    format!("common_config_{}", tool.as_str())
}

// ── Claude Code 快照切换编排（A1-hybrid）──────────────────────────────────────

/// 切走当前 Claude provider 前，把 live 的用户手改回填进该 provider 的 `meta.snapshot`。
///
/// 流程：read_live → strip_connection_env（剥离 token/base_url，明文永不落库）→
/// strip_common（剥离 common 贡献键）→ 存回 `prev_provider.meta.snapshot`。
///
/// 仅处理 Claude Code；`prev` 为空（首次切换）时是 no-op。
fn backfill_claude_snapshot(
    db: &Mutex<rusqlite::Connection>,
    config_dir: &Path,
    prev: &ProviderRow,
) -> Result<(), String> {
    let mut live = claude_snapshot::read_live(config_dir);
    claude_snapshot::strip_connection_env(&mut live);
    let common = read_common_config(db, Tool::ClaudeCode)?;
    claude_snapshot::strip_common(&mut live, common.as_ref());

    let new_meta = claude_snapshot::snapshot_into_meta(&prev.meta, &live)?;
    crate::db::dao::providers::update(
        db,
        &prev.id,
        crate::db::dao::providers::ProviderUpdate {
            meta: Some(new_meta),
            ..Default::default()
        },
    )
}

/// 把目标 Claude provider 的非连接层快照（叠加 common）整文件覆盖写入 live。
///
/// 连接层（base_url / token）不在此写入——由调用方随后的 `apply`/`apply_direct` 注入。
fn write_claude_snapshot_layer(
    db: &Mutex<rusqlite::Connection>,
    config_dir: &Path,
    target: &ProviderRow,
) -> Result<(), String> {
    let snapshot = claude_snapshot::snapshot_from_meta(&target.meta);
    let common = read_common_config(db, Tool::ClaudeCode)?;
    let enabled =
        claude_snapshot::resolve_common_enabled(&target.meta, COMMON_CONFIG_DEFAULT_ENABLED);
    let effective = claude_snapshot::build_effective(&snapshot, common.as_ref(), enabled);
    claude_snapshot::write_live_snapshot(config_dir, &effective)
}

/// Claude Code 快照切换编排（A1-hybrid）。
///
/// 相比 `enable`/`enable_direct` 的「仅连接层」语义，本函数补齐 ccs 式的
/// 回填保护 + Common Config 三层：切走前把 live 手改回填进上一个 provider 的快照，
/// 再把目标 provider 的快照层（叠加 common）整文件覆盖写 live，最后注入连接层。
///
/// `prev` 为切换前的 current provider（用于回填）；首次切换（`prev=None`）时把 live
/// 现状回填进 `target` 自身，避免用户既有 hooks/permissions 在首切时被覆盖丢失。
pub fn switch_claude(
    db: &Mutex<rusqlite::Connection>,
    data_dir: &Path,
    prev: Option<&ProviderRow>,
    target: &ProviderRow,
    crypto: Option<&CryptoService>,
) -> Result<Vec<String>, String> {
    let config_dir = resolve_config_dir(Tool::ClaudeCode)?;
    switch_claude_at(db, &config_dir, data_dir, prev, target, crypto)
}

/// `switch_claude` 的核心（接收显式 config_dir，便于用临时目录测试）。
fn switch_claude_at(
    db: &Mutex<rusqlite::Connection>,
    config_dir: &Path,
    data_dir: &Path,
    prev: Option<&ProviderRow>,
    target: &ProviderRow,
    crypto: Option<&CryptoService>,
) -> Result<Vec<String>, String> {
    if target.app_type != Tool::ClaudeCode.as_str() {
        return Err(format!(
            "provider '{}' 的 app_type '{}' 不是 claude-code",
            target.id, target.app_type
        ));
    }

    // 1. 先解析连接层（direct 需解密）。失败早退——尚未写任何文件、未回填，无副作用。
    let direct_cfg = if target.mode == "direct" {
        Some(resolve_direct_config(db, target, crypto)?)
    } else {
        None
    };

    // 2. 回填：把 live 手改捕获进「切走前 provider」的快照。
    //    首次切换无 prev → 回填进 target 自身，保护用户既有配置不被首切覆盖。
    let sink = prev.or(Some(target));
    if let Some(s) = sink {
        backfill_claude_snapshot(db, config_dir, s)?;
    }

    // 3. 备份原 live（仅首次接管，R3.4 会跳过已接管态）。
    backup_before_write(db, Tool::ClaudeCode, config_dir, data_dir)?;

    // 4. 重读 target 取最新 meta（prev==target 或首切回填 target 时 meta 已更新）。
    let fresh = crate::db::dao::providers::get(db, &target.id)?
        .ok_or_else(|| format!("provider '{}' 不存在", target.id))?;

    // 5. 写快照层（非连接键，整文件覆盖）。
    write_claude_snapshot_layer(db, config_dir, &fresh)?;

    // 6. 注入连接层（复用既有 apply/apply_direct，读-改-写叠加在快照之上）。
    match &direct_cfg {
        Some(cfg) => claude_code::apply_direct(config_dir, cfg)?,
        None => claude_code::apply(config_dir)?,
    }

    // 7. 持久化状态。
    let now = now_iso()?;
    let (mode, active, target_url) = match &direct_cfg {
        Some(cfg) => ("direct", Some(target.id.as_str()), cfg.base_url.clone()),
        None => (
            "proxy",
            None,
            format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX),
        ),
    };
    dao::upsert_state(
        db,
        Tool::ClaudeCode.as_str(),
        true,
        mode,
        active,
        Some(&now),
        Some(&target_url),
        None,
    )?;

    Ok(Vec::new())
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

    /// 插入一条 proxy 模式 provider（settings_config 空，meta 可定制三态开关等）。
    fn insert_proxy_provider(
        db: &Mutex<Connection>,
        id: &str,
        app_type: &str,
        meta: &str,
    ) -> providers::ProviderRow {
        providers::create(
            db,
            NewProvider {
                id: id.to_string(),
                app_type: app_type.to_string(),
                name: format!("prov-{}", id),
                mode: "proxy".to_string(),
                settings_config: "{}".to_string(),
                category: Some("custom".to_string()),
                sort_index: None,
                notes: None,
                meta: meta.to_string(),
            },
        )
        .unwrap()
    }

    /// 读取临时目录下 live settings.json 为 JSON。
    fn read_settings(dir: &Path) -> Value {
        let content = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        serde_json::from_str(&content).unwrap()
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

    // ── common config 读写 ─────────────────────────────────────────────────

    #[test]
    fn common_config_read_write_roundtrip() {
        let db = setup_db();
        // 未设置 → None。
        assert!(read_common_config(&db, Tool::ClaudeCode).unwrap().is_none());

        write_common_config(
            &db,
            Tool::ClaudeCode,
            &json!({ "includeCoAuthoredBy": false }),
        )
        .unwrap();
        let back = read_common_config(&db, Tool::ClaudeCode).unwrap().unwrap();
        assert_eq!(back, json!({ "includeCoAuthoredBy": false }));
    }

    #[test]
    fn write_common_config_rejects_non_object() {
        let db = setup_db();
        let err = write_common_config(&db, Tool::ClaudeCode, &json!([1, 2])).unwrap_err();
        assert!(err.contains("对象"), "非对象应报错: {}", err);
    }

    #[test]
    fn read_common_config_ignores_blank_and_non_object() {
        let db = setup_db();
        // 空串 → 视为未设置。
        crate::db::dao::app_metadata::set(&db, "common_config_claude-code", "  ").unwrap();
        assert!(read_common_config(&db, Tool::ClaudeCode).unwrap().is_none());
        // 非对象 JSON → 视为未设置。
        crate::db::dao::app_metadata::set(&db, "common_config_claude-code", "42").unwrap();
        assert!(read_common_config(&db, Tool::ClaudeCode).unwrap().is_none());
    }

    // ── switch_claude（A1-hybrid 三层编排）──────────────────────────────────

    /// 用 snapshot 构造一个含 `meta.snapshot` 的 proxy provider。
    fn insert_proxy_provider_with_snapshot(
        db: &Mutex<Connection>,
        id: &str,
        snapshot: Value,
    ) -> providers::ProviderRow {
        let meta = claude_snapshot::snapshot_into_meta("{}", &snapshot).unwrap();
        insert_proxy_provider(db, id, "claude-code", &meta)
    }

    /// AC3 + AC7(proxy)：切到 proxy provider 后，live == 快照 ⊕ common ⊕ 连接层，
    /// 且连接层为本地代理 URL + 占位 token（不降级加密）。
    #[test]
    fn switch_claude_proxy_writes_snapshot_and_connection() {
        let db = setup_db();
        let dir = unique_dir("switch-proxy");
        let a = insert_proxy_provider(&db, "a", "claude-code", "{}");
        let b = insert_proxy_provider_with_snapshot(
            &db,
            "b",
            json!({ "permissions": { "allow": ["Bash"] } }),
        );

        // 首切到 A（prev=None → 回填 A 捕获空 live），再切到 B（prev=A → 写 B 预置快照）。
        switch_claude_at(&db, &dir, &dir, None, &a, None).unwrap();
        switch_claude_at(&db, &dir, &dir, Some(&a), &b, None).unwrap();

        let live = read_settings(&dir);
        // 快照层
        assert_eq!(live["permissions"]["allow"], json!(["Bash"]));
        // 连接层：本地代理 URL + 占位 token
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap(),
            format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX)
        );
        assert_eq!(
            live["env"]["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap(),
            PLACEHOLDER_TOKEN
        );
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "proxy");
    }

    /// AC7(direct)：切到 direct provider 后 live 为真实 base_url + 解密明文 token，
    /// 且 provider 的 `meta.snapshot` 绝无明文 token（安全断言）。
    #[test]
    fn switch_claude_direct_writes_real_credentials_without_leaking() {
        let db = setup_db();
        let crypto = test_crypto();
        let dir = unique_dir("switch-direct");
        insert_endpoint(
            &db,
            &crypto,
            "ep1",
            "https://real.example.com",
            "anthropic",
            "sk-real-secret",
        );
        let seed = insert_proxy_provider(&db, "seed", "claude-code", "{}");
        let target = insert_direct_provider(
            &db,
            "p1",
            "claude-code",
            json!({ "endpoint_id": "ep1", "model": "claude-sonnet-4-6" }),
        );

        switch_claude_at(&db, &dir, &dir, None, &seed, None).unwrap();
        switch_claude_at(&db, &dir, &dir, Some(&seed), &target, Some(&crypto)).unwrap();

        let live = read_settings(&dir);
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"].as_str().unwrap(),
            "https://real.example.com"
        );
        assert_eq!(
            live["env"]["ANTHROPIC_AUTH_TOKEN"].as_str().unwrap(),
            "sk-real-secret"
        );
        let state = dao::get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(state.mode, "direct");
        assert_eq!(state.active_provider_id.as_deref(), Some("p1"));

        // 切走 direct provider → 回填其快照，明文 token 绝不落库。
        switch_claude_at(&db, &dir, &dir, Some(&target), &seed, None).unwrap();
        let p1 = providers::get(&db, "p1").unwrap().unwrap();
        assert!(
            !p1.meta.contains("sk-real-secret"),
            "backfill 后 meta 不应含明文 token: {}",
            p1.meta
        );
    }

    /// AC4：provider A live 手加 hooks → 切到 B → 切回 A，A 的 hooks 如实恢复，
    /// 且 A 的 `meta.snapshot` 无连接层占位/明文 token。
    #[test]
    fn switch_backfill_roundtrip_restores_per_provider_hooks() {
        let db = setup_db();
        let dir = unique_dir("switch-backfill");
        let a = insert_proxy_provider(&db, "a", "claude-code", "{}");
        let b = insert_proxy_provider(&db, "b", "claude-code", "{}");

        // 切到 A，用户在 live 手加 hooks。
        switch_claude_at(&db, &dir, &dir, None, &a, None).unwrap();
        let mut live = read_settings(&dir);
        live.as_object_mut()
            .unwrap()
            .insert("hooks".to_string(), json!({ "PreToolUse": "echo hi" }));
        std::fs::write(
            dir.join("settings.json"),
            serde_json::to_string_pretty(&live).unwrap(),
        )
        .unwrap();

        // 切到 B（prev=A → 回填 A 捕获 hooks）。
        switch_claude_at(&db, &dir, &dir, Some(&a), &b, None).unwrap();

        let a_row = providers::get(&db, "a").unwrap().unwrap();
        let snap = claude_snapshot::snapshot_from_meta(&a_row.meta);
        assert_eq!(snap["hooks"]["PreToolUse"], json!("echo hi"));
        assert!(
            !a_row.meta.contains(PLACEHOLDER_TOKEN),
            "快照不应含连接层占位 token: {}",
            a_row.meta
        );
        // 切到 B 后 live 不含 A 的 hooks（per-provider 隔离）。
        assert!(read_settings(&dir).get("hooks").is_none());

        // 切回 A（prev=B）→ hooks 如实恢复。
        let a_fresh = providers::get(&db, "a").unwrap().unwrap();
        switch_claude_at(&db, &dir, &dir, Some(&b), &a_fresh, None).unwrap();
        let live_back = read_settings(&dir);
        assert_eq!(live_back["hooks"]["PreToolUse"], json!("echo hi"));
    }

    /// AC5：启用 common 的 provider 切换后 live 含 common 键；三态显式 false 的
    /// provider 切换后 live 不含该键。
    #[test]
    fn switch_common_config_tristate_applies_and_skips() {
        let db = setup_db();
        let dir = unique_dir("switch-common");
        write_common_config(
            &db,
            Tool::ClaudeCode,
            &json!({ "includeCoAuthoredBy": false }),
        )
        .unwrap();

        let seed = insert_proxy_provider(&db, "seed", "claude-code", "{}");
        let enabled = insert_proxy_provider(&db, "on", "claude-code", "{}");
        let disabled = insert_proxy_provider(
            &db,
            "off",
            "claude-code",
            r#"{"common_config_enabled":false}"#,
        );

        switch_claude_at(&db, &dir, &dir, None, &seed, None).unwrap();

        // 默认（缺省=启用）→ live 含 common 键。
        switch_claude_at(&db, &dir, &dir, Some(&seed), &enabled, None).unwrap();
        assert_eq!(read_settings(&dir)["includeCoAuthoredBy"], json!(false));

        // 显式 false → live 不含 common 键。
        switch_claude_at(&db, &dir, &dir, Some(&enabled), &disabled, None).unwrap();
        assert!(read_settings(&dir).get("includeCoAuthoredBy").is_none());
    }

    /// AC6：common config 含键 X、provider 快照不含 X → 切走 backfill 后该 provider
    /// 的 `meta.snapshot` 不含 X（未被误吸收）。
    #[test]
    fn switch_backfill_strips_common_keys_from_snapshot() {
        let db = setup_db();
        let dir = unique_dir("switch-strip");
        write_common_config(
            &db,
            Tool::ClaudeCode,
            &json!({ "includeCoAuthoredBy": false }),
        )
        .unwrap();

        let seed = insert_proxy_provider(&db, "seed", "claude-code", "{}");
        let a = insert_proxy_provider_with_snapshot(&db, "a", json!({ "hooks": { "x": 1 } }));

        // 切到 A（prev=seed 保留 A 预置快照）→ live 由 common 叠加出 includeCoAuthoredBy。
        switch_claude_at(&db, &dir, &dir, None, &seed, None).unwrap();
        switch_claude_at(&db, &dir, &dir, Some(&seed), &a, None).unwrap();
        assert_eq!(read_settings(&dir)["includeCoAuthoredBy"], json!(false));

        // 切走 A（prev=A）→ backfill 应 strip 掉 common 贡献的键。
        switch_claude_at(&db, &dir, &dir, Some(&a), &seed, None).unwrap();
        let a_row = providers::get(&db, "a").unwrap().unwrap();
        let snap = claude_snapshot::snapshot_from_meta(&a_row.meta);
        assert!(
            snap.get("includeCoAuthoredBy").is_none(),
            "common 贡献的键不应被吸收进 provider 快照: {}",
            a_row.meta
        );
        // 用户自有键仍保留。
        assert_eq!(snap["hooks"]["x"], json!(1));
    }

    /// AC3(备份)：切换前若存在用户既有 settings.json，应生成 `.bak` 备份。
    #[test]
    fn switch_creates_backup_of_existing_user_settings() {
        let db = setup_db();
        let dir = unique_dir("switch-backup");
        // 预置用户既有（非接管态）settings.json。
        std::fs::write(dir.join("settings.json"), r#"{"hooks":{"user":"keep"}}"#).unwrap();
        let a = insert_proxy_provider(&db, "a", "claude-code", "{}");

        switch_claude_at(&db, &dir, &dir, None, &a, None).unwrap();

        let backup_root = dir.join("backups").join("tools");
        let has_bak = std::fs::read_dir(&backup_root)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            })
            .unwrap_or(false);
        assert!(has_bak, "切换前应生成 .bak 备份");
    }
}
