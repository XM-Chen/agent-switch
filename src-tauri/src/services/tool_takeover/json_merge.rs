//! Claude Code 切换语义（A1-hybrid）所需的 JSON 深度操作工具。
//!
//! 三层写 `settings.json` 的核心算法，语义对齐 ccs `live.rs`：
//! - [`deep_merge`]：递归合并两个 JSON，`source` 覆盖 `base`（common config 叠加用）。
//! - [`deep_remove`]：从 `target` 中递归剥离 `subset` 描述的键（backfill strip common config 用）。
//! - [`is_subset`]：判断 `subset` 是否为 `target` 的深度子集（common_config_enabled 三态检测用）。

use serde_json::Value;

/// 深度合并：把 `source` 合并进 `base`，`source` 的值覆盖 `base`。
///
/// 语义（对齐 ccs `json_deep_merge`）：
/// - 两侧同为对象 → 递归合并键。
/// - 其它情况（标量、数组、类型不一致）→ `source` 整体替换 `base`。
///   即**数组不做元素级合并**，而是整体替换。
pub fn deep_merge(base: &mut Value, source: &Value) {
    match (base, source) {
        (Value::Object(base_map), Value::Object(source_map)) => {
            for (k, v) in source_map {
                match base_map.get_mut(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        base_map.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (base_slot, source_val) => {
            *base_slot = source_val.clone();
        }
    }
}

/// 深度剥离：从 `target` 中移除 `subset` 描述的键/子树。
///
/// 语义（对齐 ccs `json_deep_remove`）：
/// - `subset` 与 `target` 同为对象时，递归下钻：
///   - 子节点两侧都是对象 → 递归剥离，剥离后子对象为空则删除该键。
///   - 否则（`subset` 侧为叶子）→ 直接删除 `target` 的该键，
///     **不比较值**（common config 贡献了这个键，无论值是否被用户覆盖都剥离）。
/// - 非对象情况不处理。
pub fn deep_remove(target: &mut Value, subset: &Value) {
    if let (Value::Object(target_map), Value::Object(subset_map)) = (target, subset) {
        for (k, sub_v) in subset_map {
            let should_remove = match target_map.get_mut(k) {
                Some(target_v) => {
                    if target_v.is_object() && sub_v.is_object() {
                        deep_remove(target_v, sub_v);
                        // 剥离后子对象变空 → 删除该键
                        target_v.as_object().map(|m| m.is_empty()).unwrap_or(false)
                    } else {
                        // subset 侧为叶子（或类型不一致）→ 直接删除
                        true
                    }
                }
                None => false,
            };
            if should_remove {
                target_map.remove(k);
            }
        }
    }
}

/// 深度子集判断：`subset` 的每个键/值是否都在 `target` 中出现且相等。
///
/// 语义（对齐 ccs `json_is_subset`）：
/// - 两侧对象 → 每个 `subset` 键都要在 `target` 存在且递归子集成立。
/// - 其它 → 值必须相等（`==`）。
///
/// 用于 legacy provider 的 `common_config_enabled` 三态检测：当该字段为 `None`
/// 时，若 provider 全文已是 common config 的超集，视为"已启用 common"。
///
/// 预留给 stage-4 common config API 的 legacy 三态检测；当前切换路径 `None` 直接
/// 视为启用，暂未调用。
#[allow(dead_code)]
pub fn is_subset(subset: &Value, target: &Value) -> bool {
    match (subset, target) {
        (Value::Object(sub_map), Value::Object(tgt_map)) => sub_map.iter().all(|(k, sub_v)| {
            tgt_map
                .get(k)
                .map(|tgt_v| is_subset(sub_v, tgt_v))
                .unwrap_or(false)
        }),
        (a, b) => a == b,
    }
}

/// 便捷构造空 JSON 对象（仅测试用）。
#[cfg(test)]
pub fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deep_merge_recurses_nested_objects() {
        let mut base = json!({
            "env": { "A": "1", "B": "2" },
            "top": "keep"
        });
        let source = json!({
            "env": { "B": "override", "C": "3" },
            "new": "added"
        });
        deep_merge(&mut base, &source);
        assert_eq!(
            base,
            json!({
                "env": { "A": "1", "B": "override", "C": "3" },
                "top": "keep",
                "new": "added"
            })
        );
    }

    #[test]
    fn deep_merge_replaces_arrays_wholesale() {
        let mut base = json!({ "list": [1, 2, 3] });
        let source = json!({ "list": [9] });
        deep_merge(&mut base, &source);
        // 数组整体替换，不做元素合并
        assert_eq!(base, json!({ "list": [9] }));
    }

    #[test]
    fn deep_merge_source_scalar_replaces_object() {
        let mut base = json!({ "x": { "nested": true } });
        let source = json!({ "x": "scalar" });
        deep_merge(&mut base, &source);
        assert_eq!(base, json!({ "x": "scalar" }));
    }

    #[test]
    fn deep_remove_strips_leaf_keys() {
        let mut target = json!({
            "includeCoAuthoredBy": false,
            "hooks": { "user": "keep" }
        });
        let subset = json!({ "includeCoAuthoredBy": false });
        deep_remove(&mut target, &subset);
        assert_eq!(target, json!({ "hooks": { "user": "keep" } }));
    }

    #[test]
    fn deep_remove_ignores_value_difference() {
        // 用户把 common config 的键改成了别的值，剥离仍应删除（因为该键源自 common）
        let mut target = json!({ "includeCoAuthoredBy": true });
        let subset = json!({ "includeCoAuthoredBy": false });
        deep_remove(&mut target, &subset);
        assert_eq!(target, json!({}));
    }

    #[test]
    fn deep_remove_recurses_and_cleans_empty_parent() {
        let mut target = json!({
            "permissions": { "allow": ["a"], "deny": ["b"] }
        });
        let subset = json!({
            "permissions": { "allow": ["a"], "deny": ["b"] }
        });
        deep_remove(&mut target, &subset);
        // 子对象被清空 → 父键也删除
        assert_eq!(target, json!({}));
    }

    #[test]
    fn deep_remove_keeps_non_common_siblings() {
        let mut target = json!({
            "permissions": { "allow": ["a"], "extra": ["keep"] }
        });
        let subset = json!({
            "permissions": { "allow": ["a"] }
        });
        deep_remove(&mut target, &subset);
        // 只剥离 common 贡献的 allow，保留用户自加的 extra
        assert_eq!(target, json!({ "permissions": { "extra": ["keep"] } }));
    }

    #[test]
    fn is_subset_true_for_nested_match() {
        let subset = json!({ "includeCoAuthoredBy": false });
        let target = json!({ "includeCoAuthoredBy": false, "other": 1 });
        assert!(is_subset(&subset, &target));
    }

    #[test]
    fn is_subset_false_when_value_differs() {
        let subset = json!({ "includeCoAuthoredBy": false });
        let target = json!({ "includeCoAuthoredBy": true });
        assert!(!is_subset(&subset, &target));
    }

    #[test]
    fn is_subset_false_when_key_missing() {
        let subset = json!({ "a": 1 });
        let target = json!({ "b": 2 });
        assert!(!is_subset(&subset, &target));
    }

    #[test]
    fn deep_merge_into_empty_base() {
        let mut base = empty_object();
        let source = json!({ "env": { "X": "1" } });
        deep_merge(&mut base, &source);
        assert_eq!(base, json!({ "env": { "X": "1" } }));
    }
}
