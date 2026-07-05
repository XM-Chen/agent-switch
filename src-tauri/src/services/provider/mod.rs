//! Provider 领域层。
//!
//! `providers` 表是 ccs 式「可切换单元」，是统一切换面。本模块提供领域类型
//! `AppType`，供上层 HTTP API 与工具接管消费。
//!
//! 与现有 `accounts`+`endpoints` 并存：
//! - proxy 模式 provider：把工具指向本地代理，上游路由仍由 endpoints 管道决定。
//! - direct 模式 provider：`settings_config` 内含真实配置，直接写工具文件绕过代理。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 切换单元所属的工具类型。
///
/// P1 仅 `ClaudeCode` / `Codex` 两种；枚举预留扩展位（Gemini/OpenCode/... 见 P2）。
/// 字符串值与 `tool_takeover::Tool` / `route_settings.id` 保持一致，避免跨表标识漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppType {
    ClaudeCode,
    Codex,
}

impl AppType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppType::ClaudeCode => "claude-code",
            AppType::Codex => "codex",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "claude-code" => Some(AppType::ClaudeCode),
            "codex" => Some(AppType::Codex),
            _ => None,
        }
    }

    /// 全部已知 app_type。
    pub fn all() -> &'static [AppType] {
        &[AppType::ClaudeCode, AppType::Codex]
    }

    /// 工具配置目录（`~/.claude` / `~/.codex`）。主目录不可用时返回 None。
    pub fn config_dir(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        match self {
            AppType::ClaudeCode => Some(home.join(".claude")),
            AppType::Codex => Some(home.join(".codex")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_type_roundtrip() {
        for t in AppType::all() {
            assert_eq!(AppType::from_str(t.as_str()), Some(*t));
        }
        assert_eq!(AppType::from_str("gemini"), None);
    }
}
