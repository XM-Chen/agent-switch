use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::error::AppError;

/// 历史应用命名空间。
///
/// 独立网关的路由决策不再由 AppType 驱动；该枚举仅用于：
/// - 旧数据库 provenance / v17 一次性迁移；
/// - 现有协议 adapter 的兼容标识；
/// - 历史 usage 日志维度。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Claude,
    #[serde(
        rename = "claude-desktop",
        alias = "claude_desktop",
        alias = "claudeDesktop"
    )]
    ClaudeDesktop,
    Codex,
    Gemini,
    OpenCode,
    OpenClaw,
    Hermes,
}

impl AppType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::OpenClaw => "openclaw",
            Self::Hermes => "hermes",
        }
    }
}

impl FromStr for AppType {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_lowercase();
        match normalized.as_str() {
            "claude" => Ok(Self::Claude),
            "claude-desktop" | "claude_desktop" | "claudedesktop" => Ok(Self::ClaudeDesktop),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            "opencode" => Ok(Self::OpenCode),
            "openclaw" => Ok(Self::OpenClaw),
            "hermes" => Ok(Self::Hermes),
            other => Err(AppError::localized(
                "unsupported_app",
                format!("不支持的应用标识: '{other}'。可选值: claude, claude-desktop, codex, gemini, opencode, openclaw, hermes。"),
                format!("Unsupported app id: '{other}'. Allowed: claude, claude-desktop, codex, gemini, opencode, openclaw, hermes."),
            )),
        }
    }
}
