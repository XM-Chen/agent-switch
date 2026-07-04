use serde_json::Value;
use std::path::Path;

use super::{
    atomic_write, DirectConfig, LiveCategory, CLAUDE_CODE_SUFFIX, LOCAL_BASE, PLACEHOLDER_TOKEN,
};

/// Claude Code 配置文件路径。
fn settings_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("settings.json")
}

/// 检测当前 Claude Code 配置指向(R5)。
///
/// 读取 `~/.claude/settings.json` → `env.ANTHROPIC_BASE_URL`。
/// - agent_switch:  等于 `LOCAL_BASE + CLAUDE_CODE_SUFFIX`
/// - official:      未设置或含 "anthropic.com"
/// - third_party:   其它非空值
/// - unconfigured:  settings.json 不存在、env 不存在或字段不存在
/// - unrecognized:  JSON 解析失败
pub fn detect(config_dir: &Path) -> LiveCategory {
    let path = settings_path(config_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return LiveCategory::Unconfigured,
    };
    let root: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return LiveCategory::Unrecognized,
    };
    let env = match root.get("env") {
        Some(Value::Object(_)) => root["env"].clone(),
        _ => return LiveCategory::Unconfigured,
    };
    let base_url = match env.get("ANTHROPIC_BASE_URL") {
        Some(Value::String(s)) => s.clone(),
        _ => return LiveCategory::Unconfigured,
    };
    if base_url.is_empty() {
        return LiveCategory::Unconfigured;
    }
    let agent_url = format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX);
    if base_url == agent_url {
        return LiveCategory::AgentSwitch;
    }
    if base_url.contains("anthropic.com") {
        return LiveCategory::Official;
    }
    LiveCategory::ThirdParty
}

/// 将 Claude Code 配置写入 agent-switch 接管态(R2.4)。
///
/// 合并写入 `settings.json`，只覆盖 `env.ANTHROPIC_BASE_URL` 与 `env.ANTHROPIC_AUTH_TOKEN`，
/// 保留文件的其它顶层键和 env 内其它键。
pub fn apply(config_dir: &Path) -> Result<(), String> {
    let path = settings_path(config_dir);
    let agent_url = format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX);

    // 读原文件(可能不存在)
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or(Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    // 确保 env 是对象
    if !root.get("env").is_some_and(|v| v.is_object()) {
        root["env"] = Value::Object(serde_json::Map::new());
    }

    if let Some(env) = root.get_mut("env") {
        if let Some(obj) = env.as_object_mut() {
            obj.insert("ANTHROPIC_BASE_URL".to_string(), Value::String(agent_url));
            obj.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                Value::String(PLACEHOLDER_TOKEN.to_string()),
            );
        }
    }

    let json_bytes = serde_json::to_vec_pretty(&root)
        .map_err(|e| format!("序列化 settings.json 失败: {}", e))?;
    atomic_write(&path, &json_bytes)?;
    Ok(())
}

/// 将 Claude Code 配置写入 direct（直连）态。
///
/// 与 `apply` 不同：写入 provider 引用端点的**真实** base_url 与解密后的 API key，
/// 使工具直连上游、绕过本地代理。`cfg.api_key` 是明文，仅用于写入文件，不记日志。
/// 合并写入（保留其它顶层键与 env 内其它键），原子写。
pub fn apply_direct(config_dir: &Path, cfg: &DirectConfig) -> Result<(), String> {
    let path = settings_path(config_dir);

    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or(Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    if !root.get("env").is_some_and(|v| v.is_object()) {
        root["env"] = Value::Object(serde_json::Map::new());
    }

    if let Some(env) = root.get_mut("env") {
        if let Some(obj) = env.as_object_mut() {
            obj.insert(
                "ANTHROPIC_BASE_URL".to_string(),
                Value::String(cfg.base_url.clone()),
            );
            obj.insert(
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                Value::String(cfg.api_key.clone()),
            );
            if let Some(model) = &cfg.model {
                obj.insert("ANTHROPIC_MODEL".to_string(), Value::String(model.clone()));
            }
        }
    }

    let json_bytes = serde_json::to_vec_pretty(&root)
        .map_err(|e| format!("序列化 settings.json 失败: {}", e))?;
    atomic_write(&path, &json_bytes)?;
    Ok(())
}
