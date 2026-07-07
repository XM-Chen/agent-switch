//! Claude Code 切换语义（A1-hybrid）的快照层。
//!
//! per-provider 的 `settings.json` **非连接键**全文快照存在 `provider.meta.snapshot`
//! （hooks / permissions / statusLine / env 内非连接键 等）。快照**不含**连接层
//! （`env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`）——连接层每次切换由 mode +
//! endpoint 重新推导，并由既有 [`super::claude_code::apply`] / [`super::claude_code::apply_direct`]
//! 注入 live。因此快照里永远没有 token，DB 天然不落明文，无需脱敏/占位符逻辑。
//!
//! 写 live 两步（见 design.md）：
//! ```text
//! 1. effective = deep_merge(snapshot, common?)  → write_live_snapshot  // 非连接层，整文件覆盖
//! 2. apply / apply_direct                                              // 连接层，读-改-写叠加
//! ```
//! backfill（切走前）：read_live → strip_connection_env → deep_remove(common) → 存回 provider.meta.snapshot。

use serde_json::{Map, Value};
use std::path::Path;

use super::atomic_write;
use super::json_merge;

/// Claude Code 配置文件名。
const SETTINGS_FILE: &str = "settings.json";

/// 连接层 env 键——快照永不保存这些，切换时由 mode+endpoint 重新注入。
const CONNECTION_ENV_KEYS: [&str; 2] = ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"];

/// 从 settings 中剥离连接层 env（`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`）。
///
/// backfill 存快照前调用：保证 `provider.meta.snapshot` 绝不含 token（明文永不落库），
/// 也不含代理 URL（连接层每次切换重新推导）。剥离后 `env` 若为空则整个删除。
pub fn strip_connection_env(settings: &mut Value) {
    let Some(obj) = settings.as_object_mut() else {
        return;
    };
    let env_empty = if let Some(env) = obj.get_mut("env").and_then(|v| v.as_object_mut()) {
        for k in CONNECTION_ENV_KEYS {
            env.remove(k);
        }
        env.is_empty()
    } else {
        false
    };
    if env_empty {
        obj.remove("env");
    }
}

/// 组装 effective settings：`deep_merge(snapshot, common)`，common 覆盖快照（source 赢）。
///
/// `common_enabled=false` 或 `common=None` 时不叠加。
pub fn build_effective(snapshot: &Value, common: Option<&Value>, common_enabled: bool) -> Value {
    let mut base = snapshot.clone();
    if common_enabled {
        if let Some(c) = common {
            json_merge::deep_merge(&mut base, c);
        }
    }
    base
}

/// 从 effective settings 中剥离 common config 贡献的键（backfill 存快照前调用）。
///
/// 避免把 common 层的键误存进 per-provider 快照，导致关闭 common 后残留。
pub fn strip_common(settings: &mut Value, common: Option<&Value>) {
    if let Some(c) = common {
        json_merge::deep_remove(settings, c);
    }
}

/// common config 三态开关的 `meta` 键名。
const META_COMMON_ENABLED_KEY: &str = "common_config_enabled";
/// per-provider 快照的 `meta` 键名。
const META_SNAPSHOT_KEY: &str = "snapshot";

/// 从 provider 的 `meta` JSON 文本中读出 per-provider 快照（`meta.snapshot`）。
///
/// meta 缺失/非对象/无 snapshot 键 → 返回空对象（等价「无自定义快照」）。
pub fn snapshot_from_meta(meta: &str) -> Value {
    serde_json::from_str::<Value>(meta)
        .ok()
        .and_then(|m| m.get(META_SNAPSHOT_KEY).cloned())
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// 把快照写回 provider 的 `meta` JSON 文本，返回更新后的 meta 字符串。
///
/// 保留 meta 里的其它键；meta 原本非对象时以空对象为基底。
pub fn snapshot_into_meta(meta: &str, snapshot: &Value) -> Result<String, String> {
    let mut root = serde_json::from_str::<Value>(meta)
        .ok()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(Map::new()));
    root.as_object_mut()
        .expect("root 已确保为对象")
        .insert(META_SNAPSHOT_KEY.to_string(), snapshot.clone());
    serde_json::to_string(&root).map_err(|e| format!("序列化 meta 失败: {}", e))
}

/// 读 provider 的 common config 三态开关（`meta.common_config_enabled`）。
///
/// 返回 `Some(true/false)` 表示用户显式设置；`None` 表示未设置（跟随默认）。
pub fn common_enabled_from_meta(meta: &str) -> Option<bool> {
    serde_json::from_str::<Value>(meta)
        .ok()
        .and_then(|m| m.get(META_COMMON_ENABLED_KEY).and_then(|v| v.as_bool()))
}

/// 解析三态开关为最终 bool：显式值优先，未设置时回落 `default_enabled`。
pub fn resolve_common_enabled(meta: &str, default_enabled: bool) -> bool {
    common_enabled_from_meta(meta).unwrap_or(default_enabled)
}

/// 把三态开关写回 provider 的 `meta` JSON 文本，返回更新后的 meta 字符串。
///
/// `Some(bool)` → 显式启用/禁用；`None` → 删除该键（回落默认）。保留 meta 其它键；
/// meta 原本非对象时以空对象为基底。
pub fn common_enabled_into_meta(meta: &str, enabled: Option<bool>) -> Result<String, String> {
    let mut root = serde_json::from_str::<Value>(meta)
        .ok()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| Value::Object(Map::new()));
    let obj = root.as_object_mut().expect("root 已确保为对象");
    match enabled {
        Some(b) => {
            obj.insert(META_COMMON_ENABLED_KEY.to_string(), Value::Bool(b));
        }
        None => {
            obj.remove(META_COMMON_ENABLED_KEY);
        }
    }
    serde_json::to_string(&root).map_err(|e| format!("序列化 meta 失败: {}", e))
}

/// 读取 live `settings.json` 全文。文件不存在或解析失败 → 返回空对象。
pub fn read_live(config_dir: &Path) -> Value {
    let path = config_dir.join(SETTINGS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or_else(|_| Value::Object(Map::new())),
        Err(_) => Value::Object(Map::new()),
    }
}

/// 整文件覆盖写入 live `settings.json`（排序键 + 原子写）。
///
/// serde_json 无 `preserve_order` feature，其 Map 为 BTreeMap，`to_vec_pretty`
/// 天然按键排序输出，等价 ccs 的排序键行为。
pub fn write_live_snapshot(config_dir: &Path, settings: &Value) -> Result<(), String> {
    let path = config_dir.join(SETTINGS_FILE);
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("序列化 settings.json 失败: {}", e))?;
    atomic_write(&path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_connection_env_removes_only_connection_keys() {
        let mut s = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://x",
                "ANTHROPIC_AUTH_TOKEN": "sk-real",
                "ANTHROPIC_MODEL": "m",
                "CLAUDE_CODE_USE_BEDROCK": "1"
            },
            "hooks": { "a": 1 }
        });
        strip_connection_env(&mut s);
        assert!(!s.to_string().contains("sk-real"), "token 明文必须被剥离");
        assert_eq!(
            s,
            json!({
                "env": { "ANTHROPIC_MODEL": "m", "CLAUDE_CODE_USE_BEDROCK": "1" },
                "hooks": { "a": 1 }
            })
        );
    }

    #[test]
    fn strip_connection_env_drops_empty_env() {
        let mut s = json!({
            "env": { "ANTHROPIC_BASE_URL": "http://x", "ANTHROPIC_AUTH_TOKEN": "t" },
            "hooks": {}
        });
        strip_connection_env(&mut s);
        // env 内只有连接键，剥离后为空 → 整个 env 删除
        assert_eq!(s, json!({ "hooks": {} }));
    }

    #[test]
    fn strip_connection_env_no_env_is_noop() {
        let mut s = json!({ "hooks": { "a": 1 } });
        strip_connection_env(&mut s);
        assert_eq!(s, json!({ "hooks": { "a": 1 } }));
    }

    #[test]
    fn build_effective_merges_common_when_enabled() {
        let snap = json!({ "env": { "ANTHROPIC_MODEL": "m" }, "includeCoAuthoredBy": true });
        let common = json!({ "includeCoAuthoredBy": false, "extra": 1 });
        let eff = build_effective(&snap, Some(&common), true);
        assert_eq!(eff["includeCoAuthoredBy"], json!(false), "common 覆盖快照");
        assert_eq!(eff["extra"], json!(1));
        assert_eq!(eff["env"]["ANTHROPIC_MODEL"], json!("m"));
    }

    #[test]
    fn build_effective_skips_common_when_disabled() {
        let snap = json!({ "includeCoAuthoredBy": true });
        let common = json!({ "includeCoAuthoredBy": false });
        let eff = build_effective(&snap, Some(&common), false);
        assert_eq!(
            eff["includeCoAuthoredBy"],
            json!(true),
            "禁用时不叠加 common"
        );
    }

    #[test]
    fn strip_common_removes_common_keys() {
        let mut eff = json!({
            "includeCoAuthoredBy": false,
            "hooks": { "user": "keep" }
        });
        let common = json!({ "includeCoAuthoredBy": false });
        strip_common(&mut eff, Some(&common));
        assert_eq!(eff, json!({ "hooks": { "user": "keep" } }));
    }

    #[test]
    fn write_and_read_live_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "as-snapshot-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = json!({ "env": { "ANTHROPIC_MODEL": "m" }, "hooks": { "a": 1 } });
        write_live_snapshot(&dir, &settings).unwrap();
        let back = read_live(&dir);
        assert_eq!(back, settings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_live_missing_returns_empty_object() {
        let dir = std::env::temp_dir().join(format!("as-snapshot-missing-{}", std::process::id()));
        let v = read_live(&dir);
        assert_eq!(v, json!({}));
    }

    #[test]
    fn snapshot_meta_roundtrip_preserves_other_keys() {
        let meta = r#"{"common_config_enabled":true,"other":"keep"}"#;
        let snap = json!({ "hooks": { "a": 1 } });
        let updated = snapshot_into_meta(meta, &snap).unwrap();
        // snapshot 写入且其它键保留
        assert_eq!(snapshot_from_meta(&updated), snap);
        let root: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(root["other"], json!("keep"));
        assert_eq!(root["common_config_enabled"], json!(true));
    }

    #[test]
    fn snapshot_from_meta_absent_returns_empty() {
        assert_eq!(snapshot_from_meta("{}"), json!({}));
        assert_eq!(snapshot_from_meta("not json"), json!({}));
        assert_eq!(
            snapshot_from_meta(r#"{"snapshot":{"x":1}}"#),
            json!({ "x": 1 })
        );
    }

    #[test]
    fn snapshot_into_meta_from_empty_base() {
        let updated = snapshot_into_meta("{}", &json!({ "x": 1 })).unwrap();
        assert_eq!(snapshot_from_meta(&updated), json!({ "x": 1 }));
    }

    #[test]
    fn common_enabled_tristate() {
        assert_eq!(
            common_enabled_from_meta(r#"{"common_config_enabled":true}"#),
            Some(true)
        );
        assert_eq!(
            common_enabled_from_meta(r#"{"common_config_enabled":false}"#),
            Some(false)
        );
        assert_eq!(common_enabled_from_meta("{}"), None);
        // 未设置 → 回落默认
        assert!(resolve_common_enabled("{}", true));
        assert!(!resolve_common_enabled("{}", false));
        // 显式值优先于默认
        assert!(!resolve_common_enabled(
            r#"{"common_config_enabled":false}"#,
            true
        ));
        assert!(resolve_common_enabled(
            r#"{"common_config_enabled":true}"#,
            false
        ));
    }

    #[test]
    fn common_enabled_into_meta_sets_and_clears() {
        // 显式启用/禁用写入。
        let m = common_enabled_into_meta("{}", Some(true)).unwrap();
        assert_eq!(common_enabled_from_meta(&m), Some(true));
        let m = common_enabled_into_meta(&m, Some(false)).unwrap();
        assert_eq!(common_enabled_from_meta(&m), Some(false));
        // None → 删除键，回落默认（未设置）。
        let m = common_enabled_into_meta(&m, None).unwrap();
        assert_eq!(common_enabled_from_meta(&m), None);
    }

    #[test]
    fn common_enabled_into_meta_preserves_other_keys() {
        let meta = r#"{"snapshot":{"hooks":{"a":1}},"other":"keep"}"#;
        let updated = common_enabled_into_meta(meta, Some(false)).unwrap();
        // 三态写入且 snapshot/其它键保留。
        assert_eq!(common_enabled_from_meta(&updated), Some(false));
        assert_eq!(snapshot_from_meta(&updated), json!({ "hooks": { "a": 1 } }));
        let root: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(root["other"], json!("keep"));
    }
}
