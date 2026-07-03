//! Provider 领域层。
//!
//! `providers` 表是 ccs 式「可切换单元」，是统一切换面。本模块提供领域类型
//! （`Provider` / `AppType` / `ProviderMode` / `ProviderMeta`）与 DAO 行之间的
//! 转换，供上层 HTTP API 与工具接管消费。
//!
//! 与现有 `accounts`+`endpoints` 并存：
//! - proxy 模式 provider：把工具指向本地代理，上游路由仍由 endpoints 管道决定。
//! - direct 模式 provider：`settings_config` 内含真实配置，直接写工具文件绕过代理。
//!
//! 领域方法将在后续子任务（双模式接管 / Provider CRUD API）逐步接线；
//! 接线完成前由 `services/mod.rs` 的 `#[allow(dead_code)]` 覆盖，与 translator 先例一致。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use crate::db::dao::providers::{NewProvider, ProviderRow};

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

/// Provider 接管模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderMode {
    /// 工具指向本地代理，上游由 endpoints 管道路由（默认，保留 agent-switch 定位）。
    #[default]
    Proxy,
    /// 工具直连真实上游，绕过代理（无 failover/翻译语义）。
    Direct,
}

impl ProviderMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderMode::Proxy => "proxy",
            ProviderMode::Direct => "direct",
        }
    }

    /// 解析字符串；未知值回退到 `Proxy`（代理优先）。
    pub fn from_str_or_proxy(s: &str) -> Self {
        match s {
            "direct" => ProviderMode::Direct,
            _ => ProviderMode::Proxy,
        }
    }
}

/// 不写入 live 配置的 provider 元数据。
///
/// 未知字段收进 `extra`，避免升级时丢弃前向新增的元数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderMeta {
    /// 供应商官网/文档链接。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub website_url: Option<String>,
    /// 图标标识（前端渲染用）。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,
    /// 图标颜色。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon_color: Option<String>,
    /// 保留未识别字段，前向兼容。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl ProviderMeta {
    /// 从 JSON 字符串解析；空/非法回退为默认空 meta。
    pub fn from_json_str(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    /// 序列化为 JSON 字符串（存 DB）。
    pub fn to_json_string(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("序列化 meta 失败: {}", e))
    }
}

/// Provider 领域对象（DB 行 + 解析后的强类型视图）。
#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    pub id: String,
    pub app_type: String,
    pub name: String,
    pub mode: ProviderMode,
    /// 工具原生配置或代理指向配置（解析后的 JSON 值）。
    pub settings_config: Value,
    pub is_current: bool,
    pub category: Option<String>,
    pub sort_index: Option<i64>,
    pub notes: Option<String>,
    pub meta: ProviderMeta,
    pub created_at: String,
    pub updated_at: String,
}

impl Provider {
    /// 从 DAO 行构建领域对象。`settings_config` / `meta` 的 JSON 解析失败时回退到
    /// 空对象/默认，避免单条脏数据导致整表读取失败。
    pub fn from_row(row: ProviderRow) -> Self {
        let settings_config =
            serde_json::from_str(&row.settings_config).unwrap_or(Value::Object(Default::default()));
        let meta = ProviderMeta::from_json_str(&row.meta);
        Provider {
            id: row.id,
            app_type: row.app_type,
            name: row.name,
            mode: ProviderMode::from_str_or_proxy(&row.mode),
            settings_config,
            is_current: row.is_current,
            category: row.category,
            sort_index: row.sort_index,
            notes: row.notes,
            meta,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    /// 解析强类型 `AppType`（未知返回 None）。
    pub fn app_type_enum(&self) -> Option<AppType> {
        AppType::from_str(&self.app_type)
    }
}

/// 创建 provider 的领域输入（上层构造，转换为 DAO `NewProvider`）。
#[derive(Debug, Clone)]
pub struct NewProviderInput {
    pub id: String,
    pub app_type: AppType,
    pub name: String,
    pub mode: ProviderMode,
    pub settings_config: Value,
    pub category: Option<String>,
    pub sort_index: Option<i64>,
    pub notes: Option<String>,
    pub meta: ProviderMeta,
}

impl NewProviderInput {
    /// 转换为 DAO 层输入，序列化 JSON 字段。
    pub fn into_dao(self) -> Result<NewProvider, String> {
        let settings_config = serde_json::to_string(&self.settings_config)
            .map_err(|e| format!("序列化 settings_config 失败: {}", e))?;
        let meta = self.meta.to_json_string()?;
        Ok(NewProvider {
            id: self.id,
            app_type: self.app_type.as_str().to_string(),
            name: self.name,
            mode: self.mode.as_str().to_string(),
            settings_config,
            category: self.category,
            sort_index: self.sort_index,
            notes: self.notes,
            meta,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_type_roundtrip() {
        for t in AppType::all() {
            assert_eq!(AppType::from_str(t.as_str()), Some(*t));
        }
        assert_eq!(AppType::from_str("gemini"), None);
    }

    #[test]
    fn provider_mode_defaults_to_proxy() {
        assert_eq!(ProviderMode::default(), ProviderMode::Proxy);
        assert_eq!(
            ProviderMode::from_str_or_proxy("unknown"),
            ProviderMode::Proxy
        );
        assert_eq!(
            ProviderMode::from_str_or_proxy("direct"),
            ProviderMode::Direct
        );
    }

    #[test]
    fn meta_preserves_unknown_fields() {
        let raw = r#"{"website_url":"https://x.ai","future_flag":true}"#;
        let meta = ProviderMeta::from_json_str(raw);
        assert_eq!(meta.website_url.as_deref(), Some("https://x.ai"));
        assert_eq!(meta.extra.get("future_flag"), Some(&json!(true)));
        // round-trip 不丢失未知字段
        let out = meta.to_json_string().unwrap();
        assert!(out.contains("future_flag"));
    }

    #[test]
    fn meta_empty_or_invalid_falls_back() {
        assert!(ProviderMeta::from_json_str("").extra.is_empty());
        assert!(ProviderMeta::from_json_str("not json").extra.is_empty());
        assert!(ProviderMeta::from_json_str("{}").website_url.is_none());
    }

    #[test]
    fn from_row_parses_json_fields() {
        let row = ProviderRow {
            id: "p1".to_string(),
            app_type: "claude-code".to_string(),
            name: "Test".to_string(),
            mode: "direct".to_string(),
            settings_config: r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#
                .to_string(),
            is_current: true,
            category: Some("official".to_string()),
            sort_index: Some(3),
            notes: None,
            meta: r#"{"icon":"anthropic"}"#.to_string(),
            created_at: "2026-07-04T00:00:00Z".to_string(),
            updated_at: "2026-07-04T00:00:00Z".to_string(),
        };
        let p = Provider::from_row(row);
        assert_eq!(p.mode, ProviderMode::Direct);
        assert_eq!(p.app_type_enum(), Some(AppType::ClaudeCode));
        assert_eq!(p.meta.icon.as_deref(), Some("anthropic"));
        assert_eq!(
            p.settings_config["env"]["ANTHROPIC_BASE_URL"],
            json!("https://api.anthropic.com")
        );
    }

    #[test]
    fn from_row_tolerates_bad_json() {
        let row = ProviderRow {
            id: "p1".to_string(),
            app_type: "codex".to_string(),
            name: "Bad".to_string(),
            mode: "proxy".to_string(),
            settings_config: "not json".to_string(),
            is_current: false,
            category: None,
            sort_index: None,
            notes: None,
            meta: "also bad".to_string(),
            created_at: "2026-07-04T00:00:00Z".to_string(),
            updated_at: "2026-07-04T00:00:00Z".to_string(),
        };
        let p = Provider::from_row(row);
        assert!(p.settings_config.is_object());
        assert!(p.meta.extra.is_empty());
    }

    #[test]
    fn new_input_into_dao_serializes() {
        let input = NewProviderInput {
            id: "p1".to_string(),
            app_type: AppType::Codex,
            name: "Test".to_string(),
            mode: ProviderMode::Proxy,
            settings_config: json!({"base_url": "http://127.0.0.1:42567/codex"}),
            category: Some("custom".to_string()),
            sort_index: Some(0),
            notes: None,
            meta: ProviderMeta::default(),
        };
        let dao = input.into_dao().unwrap();
        assert_eq!(dao.app_type, "codex");
        assert_eq!(dao.mode, "proxy");
        assert!(dao.settings_config.contains("42567"));
        assert_eq!(dao.meta, "{}");
    }
}
