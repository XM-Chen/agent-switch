use serde_json::Value;
use std::path::Path;

use super::{
    atomic_write, DirectConfig, LiveCategory, CODEX_PROVIDER_ID, CODEX_SUFFIX, LOCAL_BASE,
    PLACEHOLDER_TOKEN,
};

/// Codex config.toml 路径。
fn toml_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("config.toml")
}

/// Codex auth.json 路径。
fn auth_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("auth.json")
}

/// 检测当前 Codex 配置指向(R5)。
///
/// 读取 `~/.codex/config.toml` → `model_provider` + `[model_providers.*].base_url`。
/// - agent_switch:  `model_provider == CODEX_PROVIDER_ID`
/// - official:      文件不存在或未设置自定义 provider
/// - third_party:   其它 provider
/// - unconfigured:  TOML 缺失/无 model_provider
/// - unrecognized:  解析失败
pub fn detect(config_dir: &Path) -> LiveCategory {
    let path = toml_path(config_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return LiveCategory::Unconfigured,
    };
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return LiveCategory::Unrecognized,
    };

    let mp = match doc.get("model_provider") {
        Some(v) => v.as_str().unwrap_or("").to_string(),
        None => return LiveCategory::Unconfigured,
    };

    if mp == CODEX_PROVIDER_ID {
        return LiveCategory::AgentSwitch;
    }
    let provider_table_key = format!("model_providers.{}", mp);
    if doc.get(&provider_table_key).is_none() {
        // model_provider 指向的 provider 表不存在 → 视为官方
        return LiveCategory::Official;
    }
    LiveCategory::ThirdParty
}

/// 将 Codex 配置写入 agent-switch 接管态(R2.5)。
///
/// 合并写入 `config.toml`(toml_edit)与 `auth.json`(serde_json)。
pub fn apply(config_dir: &Path) -> Result<(), String> {
    apply_toml(config_dir)?;
    apply_auth(config_dir)?;
    Ok(())
}

/// 写入 `config.toml`：
/// - 顶层 `model_provider = "agent-switch"`
/// - `[model_providers.agent-switch]` 表：
///   `name`, `base_url`, `wire_api = "responses"`, `requires_openai_auth = true`
/// - 保留用户其他配置与其它 provider 表。
fn apply_toml(config_dir: &Path) -> Result<(), String> {
    let path = toml_path(config_dir);
    let agent_url = format!("{}{}", LOCAL_BASE, CODEX_SUFFIX);

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    // 设置顶层 model_provider
    doc["model_provider"] = toml_edit::value(CODEX_PROVIDER_ID);

    // 确保 [model_providers.agent-switch] 表存在
    let provider_key = format!("model_providers.{}", CODEX_PROVIDER_ID);
    if !doc.as_table().contains_key(&provider_key) {
        doc[&provider_key] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc[&provider_key]["name"] = toml_edit::value(CODEX_PROVIDER_ID);
    doc[&provider_key]["base_url"] = toml_edit::value(&agent_url);
    doc[&provider_key]["wire_api"] = toml_edit::value("responses");
    doc[&provider_key]["requires_openai_auth"] = toml_edit::value(true);

    let out = doc.to_string();
    atomic_write(&path, out.as_bytes())?;
    Ok(())
}

/// 写入 `auth.json`：只覆盖 `OPENAI_API_KEY` 为占位符，
/// 保留 `tokens`、`last_refresh` 等字段(若有)。
fn apply_auth(config_dir: &Path) -> Result<(), String> {
    let path = auth_path(config_dir);

    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or(Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    root["OPENAI_API_KEY"] = Value::String(PLACEHOLDER_TOKEN.to_string());

    let json_bytes =
        serde_json::to_vec_pretty(&root).map_err(|e| format!("序列化 auth.json 失败: {}", e))?;
    atomic_write(&path, &json_bytes)?;
    Ok(())
}

/// 将 Codex 配置写入 direct（直连）态。
///
/// 与 `apply` 不同：`config.toml` 写入 provider 引用端点的**真实** base_url，
/// provider id 取自 `cfg.provider_id`（作 `model_provider` 与表名）；`auth.json`
/// 写入解密后的真实 API key。`cfg.api_key` 明文仅用于写文件，不记日志。
/// 合并写入（保留其它 provider 表与 auth.json 其它字段），原子写。
pub fn apply_direct(config_dir: &Path, cfg: &DirectConfig) -> Result<(), String> {
    apply_direct_toml(config_dir, cfg)?;
    apply_direct_auth(config_dir, cfg)?;
    Ok(())
}

/// 写入 `config.toml` direct 态：
/// - 顶层 `model_provider = <provider_id>`
/// - `[model_providers.<provider_id>]`：`name`、`base_url`(真实)、
///   `wire_api`(缺省 responses)、`requires_openai_auth`(缺省 true)
/// - 保留用户其它配置与其它 provider 表。
fn apply_direct_toml(config_dir: &Path, cfg: &DirectConfig) -> Result<(), String> {
    let path = toml_path(config_dir);

    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    doc["model_provider"] = toml_edit::value(&cfg.provider_id);

    let provider_key = format!("model_providers.{}", cfg.provider_id);
    if !doc.as_table().contains_key(&provider_key) {
        doc[&provider_key] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc[&provider_key]["name"] = toml_edit::value(&cfg.provider_id);
    doc[&provider_key]["base_url"] = toml_edit::value(&cfg.base_url);
    doc[&provider_key]["wire_api"] =
        toml_edit::value(cfg.wire_api.as_deref().unwrap_or("responses"));
    doc[&provider_key]["requires_openai_auth"] =
        toml_edit::value(cfg.requires_openai_auth.unwrap_or(true));

    if let Some(model) = &cfg.model {
        doc["model"] = toml_edit::value(model);
    }

    let out = doc.to_string();
    atomic_write(&path, out.as_bytes())?;
    Ok(())
}

/// 写入 `auth.json` direct 态：覆盖 `OPENAI_API_KEY` 为真实 key，保留其它字段。
fn apply_direct_auth(config_dir: &Path, cfg: &DirectConfig) -> Result<(), String> {
    let path = auth_path(config_dir);

    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or(Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    root["OPENAI_API_KEY"] = Value::String(cfg.api_key.clone());

    let json_bytes =
        serde_json::to_vec_pretty(&root).map_err(|e| format!("序列化 auth.json 失败: {}", e))?;
    atomic_write(&path, &json_bytes)?;
    Ok(())
}
