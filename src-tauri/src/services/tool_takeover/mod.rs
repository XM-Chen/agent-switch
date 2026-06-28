pub mod claude_code;
pub mod codex;

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::db::dao::tool_takeover as dao;

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

// ── Public API ──────────────────────────────────────────────────────────────

/// 启用接管——备份 + 写入工具配置 + 持久化成功状态。
pub fn enable(db: &Mutex<rusqlite::Connection>, tool: Tool, data_dir: &Path) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }
    let config_dir = tool
        .config_dir()
        .ok_or_else(|| "无法获取用户主目录".to_string())?;

    // 1. 备份原配置(R3,R3.4)
    backup_before_write(db, tool, &config_dir, data_dir)?;

    // 2. 写入工具配置，指向 agent-switch
    match tool {
        Tool::ClaudeCode => claude_code::apply(&config_dir)?,
        Tool::Codex => codex::apply(&config_dir)?,
        _ => unreachable!(),
    }

    // 3. 持久化成功状态
    let target = match tool {
        Tool::ClaudeCode => format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX),
        Tool::Codex => format!("{}{}", LOCAL_BASE, CODEX_SUFFIX),
        _ => unreachable!(),
    };
    let now = iso_now()?;
    dao::upsert_state(db, tool.as_str(), true, Some(&now), Some(&target), None)?;

    Ok(())
}

/// 关闭接管——只改开关，不还原工具文件(R7)。
pub fn disable(db: &Mutex<rusqlite::Connection>, tool: Tool) -> Result<(), String> {
    if !tool.supports_takeover() {
        return Err(format!("工具 '{}' 不支持自动接管", tool.as_str()));
    }
    dao::set_enabled(db, tool.as_str(), false)?;
    Ok(())
}

/// 重新应用接管(R4.2，幂等)。要求已开启接管。
pub fn reapply(
    db: &Mutex<rusqlite::Connection>,
    tool: Tool,
    data_dir: &Path,
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
    enable(db, tool, data_dir)
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

    let timestamp = iso_now()?;
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

fn iso_now() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}
