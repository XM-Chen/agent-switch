//! 聚合派生计算（CC 聚合功能 C2）
//!
//! 纯计算分层（与 `usage_stats` 一致），零网络。在 C1 的 `provider_models` 缓存之上
//! 派生两样东西：
//! - **自动聚合**（`build_aggregates`）：纯派生视图 `聚合 = f(队列顺序 × 每上游模型缓存)`，
//!   不持久化、每次现算（D1/D2/D4）。
//! - **自定义聚合**（`build_custom_aggregates`）：持久化定义 + 现算内容（D7）。
//!
//! 并提供统一展平 API（`flatten_aggregate_ref`），把任一可路由聚合展平成有序
//! `(上游, 模型 id)` 候选序列，供 C3 路由复用。

use crate::database::{AggregateRef, Database};
use crate::error::AppError;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// 一个聚合内的一个上游候选。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AggregateMember {
    pub provider_id: String,
    pub provider_name: String,
    /// 该上游行返回的 model id 原文（用于路由改写，必须精确透传）。
    pub model_id: String,
    /// `fetched` | `manual`（透传，供 UI 标记）。
    pub source: String,
}

/// 一个自动聚合。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AggregateView {
    /// 聚合 key = 桶内首次出现的 model_id 原文（display_id，保留大小写，D2）。
    pub key: String,
    /// 已按故障转移队列序 P1→P2 排列的上游候选。
    pub members: Vec<AggregateMember>,
}

/// 一个自定义聚合的派生视图（区别于持久化的 `CustomAggregate` 行）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomAggregateView {
    pub id: String,
    pub name: String,
    /// 展平后的候选，按 D7「外层成员序 × 内层上游序」排列并去重。
    pub members: Vec<AggregateMember>,
    /// 归零标记（D7：只标记不删）。全部成员归零 → `true`。
    pub is_empty: bool,
    /// `ordered_members` 中当前已归零/不存在的自动聚合 key（原样保留用户输入）。
    pub missing_members: Vec<String>,
}

/// 展平后的单个路由候选（供 C3 路由消费）。
///
/// C2 提供、C3 消费；在 C3 落地前无库内消费方，故 `allow(dead_code)`。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteCandidate {
    pub provider_id: String,
    pub model_id: String,
}

/// 构建「桶键 → 自动聚合」的有序映射（内部复用）。
///
/// - 桶键 `bucket_key = model_id.to_lowercase()`，**仅用于判定是否同一聚合**（D2）。
/// - `AggregateView.key = display_id`（桶内首次出现的原文），存储/展示/路由一律用它，
///   绝不用 lowercase 覆盖原文。
/// - 候选来源严格 = 故障转移队列成员（D4/D5）：不在队列的上游即使有缓存也不进聚合，
///   靠 `get_failover_queue` 过滤实现，无需删缓存。
/// - 遍历顺序 = 队列序 P1→P2，故桶内成员天然按队列序排列（D4）。
fn build_aggregate_map(
    db: &Database,
    app_type: &str,
) -> Result<IndexMap<String, AggregateView>, AppError> {
    // P1→P2 顺序的队列成员。
    let queue = db.get_failover_queue(app_type)?;
    // provider 名称查表（队列条目已带 name，但保持与 design 一致仍走此表兜底）。
    let providers = db.get_all_providers(app_type)?;

    // 桶键（lowercase）→ 聚合视图；IndexMap 保留首次出现顺序（D1 稳定排序）。
    let mut buckets: IndexMap<String, AggregateView> = IndexMap::new();

    for item in &queue {
        let provider_id = &item.provider_id;
        // 优先用 providers 表的 name；队列条目 name 作兜底。
        let provider_name = providers
            .get(provider_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| item.provider_name.clone());

        let models = db.list_provider_models(app_type, Some(provider_id))?;
        for model in models {
            let bucket_key = model.model_id.to_lowercase();
            let member = AggregateMember {
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                model_id: model.model_id.clone(), // display_id / 路由原文
                source: model.source.clone(),
            };
            buckets
                .entry(bucket_key)
                .or_insert_with(|| AggregateView {
                    // 桶内首次出现的原文作为 key（保留大小写）。
                    key: model.model_id.clone(),
                    members: Vec::new(),
                })
                .members
                .push(member);
        }
    }

    Ok(buckets)
}

/// 自动聚合派生（D1/D2/D4）。读 `provider_models` + `get_failover_queue`，纯派生。
///
/// 空桶不存在（归零自动消失 = 自动聚合语义 D1/D6）。
pub fn build_aggregates(db: &Database, app_type: &str) -> Result<Vec<AggregateView>, AppError> {
    Ok(build_aggregate_map(db, app_type)?.into_values().collect())
}

/// 把候选去重：同一 `(provider_id, model_id)` 只保留首次出现（R17）。
fn dedup_members(members: Vec<AggregateMember>) -> Vec<AggregateMember> {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(members.len());
    for m in members {
        if seen.insert((m.provider_id.clone(), m.model_id.clone())) {
            out.push(m);
        }
    }
    out
}

/// 自定义聚合派生（D7）。定义来自 `custom_aggregates` 表，内容现算。
///
/// - 按 `ordered_members` 顺序取对应自动聚合，「外层成员序 × 内层上游序」展平并去重。
/// - 缺失/归零的成员 key 记入 `missing_members`（R12）。
/// - `is_empty = members.is_empty()`（R13：只标记不删，后端绝不自动删定义）。
pub fn build_custom_aggregates(
    db: &Database,
    app_type: &str,
) -> Result<Vec<CustomAggregateView>, AppError> {
    let auto_map = build_aggregate_map(db, app_type)?;
    let definitions = db.list_custom_aggregates(app_type)?;

    let mut views = Vec::with_capacity(definitions.len());
    for def in definitions {
        let mut members: Vec<AggregateMember> = Vec::new();
        let mut missing: Vec<String> = Vec::new();

        for member_key in &def.ordered_members {
            // 大小写无损查找：成员存的是 display_id 原文，用 lowercase 桶键匹配，
            // 兼容首次出现原文随刷新漂移的情况。
            match auto_map.get(&member_key.to_lowercase()) {
                Some(view) if !view.members.is_empty() => {
                    members.extend(view.members.iter().cloned());
                }
                // 不存在或归零：记入 missing（保留用户原始输入）。
                _ => missing.push(member_key.clone()),
            }
        }

        let members = dedup_members(members);
        views.push(CustomAggregateView {
            id: def.id,
            name: def.name,
            is_empty: members.is_empty(),
            members,
            missing_members: missing,
        });
    }

    Ok(views)
}

/// 把一个聚合引用（自动 key 或自定义 id）展平成有序候选（D7 统一路由抽象，供 C3 复用）。
///
/// - 自动聚合：直接取其 members（单成员特例）。
/// - 自定义聚合：按 `ordered_members`「外层成员序 × 内层上游序」拼接并去重。
/// - 未知 ref（指向已删的自定义聚合 / 已归零的自动 key）→ 返回空 Vec + warn，
///   由 C3 决定降级。
///
/// C2 提供、C3 消费；在 C3 落地前无库内消费方，故 `allow(dead_code)`。
#[allow(dead_code)]
pub fn flatten_aggregate_ref(
    db: &Database,
    app_type: &str,
    aggregate_ref: &AggregateRef,
) -> Result<Vec<RouteCandidate>, AppError> {
    let auto_map = build_aggregate_map(db, app_type)?;

    let members: Vec<AggregateMember> = match aggregate_ref {
        AggregateRef::Auto(key) => match auto_map.get(&key.to_lowercase()) {
            Some(view) => view.members.clone(),
            None => {
                log::warn!("flatten_aggregate_ref: 自动聚合 key `{key}` 已归零或不存在");
                Vec::new()
            }
        },
        AggregateRef::Custom(id) => {
            let definitions = db.list_custom_aggregates(app_type)?;
            match definitions.into_iter().find(|d| &d.id == id) {
                Some(def) => {
                    let mut acc: Vec<AggregateMember> = Vec::new();
                    for member_key in &def.ordered_members {
                        if let Some(view) = auto_map.get(&member_key.to_lowercase()) {
                            acc.extend(view.members.iter().cloned());
                        }
                    }
                    dedup_members(acc)
                }
                None => {
                    log::warn!("flatten_aggregate_ref: 自定义聚合 id `{id}` 不存在");
                    Vec::new()
                }
            }
        }
    };

    Ok(members
        .into_iter()
        .map(|m| RouteCandidate {
            provider_id: m.provider_id,
            model_id: m.model_id,
        })
        .collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rusqlite::params;

    const APP: &str = "claude";

    /// 插入一个 provider 并（可选）加入故障转移队列，设置 sort_index 决定队列序。
    fn insert_provider(db: &Database, id: &str, in_queue: bool, sort_index: i64) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta, is_current, in_failover_queue, sort_index)
             VALUES (?1, ?2, ?3, '{}', '{}', 0, ?4, ?5)",
            params![id, APP, id, if in_queue { 1 } else { 0 }, sort_index],
        )
        .unwrap();
    }

    fn fetch(db: &Database, provider_id: &str, model_id: &str) {
        use crate::services::model_fetch::FetchedModel;
        db.replace_fetched_models(
            APP,
            provider_id,
            &[FetchedModel {
                id: model_id.to_string(),
                owned_by: None,
            }],
            100,
        )
        .unwrap();
    }

    fn find<'a>(views: &'a [AggregateView], key: &str) -> Option<&'a AggregateView> {
        views.iter().find(|v| v.key == key)
    }

    // D2：同一上游返回三种写法 → 三个独立聚合（bucket_key 不同）。
    #[test]
    fn d2_same_upstream_distinct_ids_stay_separate() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        use crate::services::model_fetch::FetchedModel;
        db.replace_fetched_models(
            APP,
            "p1",
            &[
                FetchedModel { id: "glm-4.6".into(), owned_by: None },
                FetchedModel { id: "zhipu/glm-4.6".into(), owned_by: None },
                FetchedModel { id: "z-ai/glm-4.6".into(), owned_by: None },
            ],
            100,
        )
        .unwrap();

        let views = build_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 3, "三种不同 id 应成三个独立聚合");
        assert!(find(&views, "glm-4.6").is_some());
        assert!(find(&views, "zhipu/glm-4.6").is_some());
        assert!(find(&views, "z-ai/glm-4.6").is_some());
    }

    // D2：跨上游大小写不同 → 归入同一聚合，display_id 取首次出现原文。
    #[test]
    fn d2_cross_upstream_case_insensitive_merge() {
        let db = Database::memory().unwrap();
        // p1 先出现（sort_index 0），返回大写 GLM-4.6 → display_id。
        insert_provider(&db, "p1", true, 0);
        insert_provider(&db, "p2", true, 1);
        fetch(&db, "p1", "GLM-4.6");
        fetch(&db, "p2", "glm-4.6");

        let views = build_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 1, "大小写不同应归入同一聚合");
        let view = &views[0];
        assert_eq!(view.key, "GLM-4.6", "display_id 取首次出现原文（p1 的大写）");
        assert_eq!(view.members.len(), 2);
        // 队列序：p1 在前。
        assert_eq!(view.members[0].provider_id, "p1");
        assert_eq!(view.members[0].model_id, "GLM-4.6");
        // 成员各自保留原文，路由改写时发给上游的名字精确。
        assert_eq!(view.members[1].provider_id, "p2");
        assert_eq!(view.members[1].model_id, "glm-4.6");
    }

    // D4：内部候选按故障转移队列序 P1→P2 排列。
    #[test]
    fn d4_members_follow_queue_order() {
        let db = Database::memory().unwrap();
        // 故意让 sort_index 决定队列序：p2 (0) 在 p1 (1) 之前。
        insert_provider(&db, "p1", true, 1);
        insert_provider(&db, "p2", true, 0);
        fetch(&db, "p1", "shared");
        fetch(&db, "p2", "shared");

        let views = build_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 1);
        let members = &views[0].members;
        assert_eq!(members[0].provider_id, "p2", "sort_index 更小的 p2 应在前");
        assert_eq!(members[1].provider_id, "p1");
    }

    // D4/D5：不在队列的上游即使有缓存也不进聚合。
    #[test]
    fn d4_non_queue_provider_excluded() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        insert_provider(&db, "p2", false, 1); // 有缓存但不在队列
        fetch(&db, "p1", "m");
        fetch(&db, "p2", "m");

        let views = build_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].members.len(), 1, "只有队列成员 p1 进聚合");
        assert_eq!(views[0].members[0].provider_id, "p1");
    }

    // D1：归零自动聚合不出现（移出队列后重算）。
    #[test]
    fn d1_zeroed_aggregate_disappears() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        fetch(&db, "p1", "m");
        assert_eq!(build_aggregates(&db, APP).unwrap().len(), 1);

        // 移出队列 → 重算时聚合归零消失。
        db.remove_from_failover_queue(APP, "p1").unwrap();
        assert!(build_aggregates(&db, APP).unwrap().is_empty());
    }

    // D6：手动模型参与派生，出现在对应自动聚合中。
    #[test]
    fn d6_manual_model_participates() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        db.upsert_manual_model(APP, "p1", "manual-x", 100).unwrap();

        let views = build_aggregates(&db, APP).unwrap();
        let view = find(&views, "manual-x").expect("手动模型应派生出聚合");
        assert_eq!(view.members[0].source, "manual");
    }

    // D7：CCC=[C,D] 展平 = [p2→C, p3→C, p3→D]（先试完 C 的所有上游，再试 D）。
    #[test]
    fn d7_flatten_order_outer_member_inner_upstream() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p2", true, 0);
        insert_provider(&db, "p3", true, 1);
        // C 由 p2、p3 提供；D 只由 p3 提供。
        fetch(&db, "p2", "C");
        use crate::services::model_fetch::FetchedModel;
        db.replace_fetched_models(
            APP,
            "p3",
            &[
                FetchedModel { id: "C".into(), owned_by: None },
                FetchedModel { id: "D".into(), owned_by: None },
            ],
            100,
        )
        .unwrap();

        let id = db.create_custom_aggregate(APP, "CCC", &["C".into(), "D".into()]).unwrap();
        let flat = flatten_aggregate_ref(&db, APP, &AggregateRef::Custom(id)).unwrap();

        let seq: Vec<(String, String)> = flat
            .iter()
            .map(|c| (c.provider_id.clone(), c.model_id.clone()))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("p2".into(), "C".into()),
                ("p3".into(), "C".into()),
                ("p3".into(), "D".into()),
            ],
            "外层成员序 × 内层上游序"
        );
    }

    // D7：删 p3 → CCC 只剩 C（p2）；再删 p2 → is_empty=true 且定义仍在。
    #[test]
    fn d7_custom_shrinks_with_members_and_never_auto_deletes() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p2", true, 0);
        insert_provider(&db, "p3", true, 1);
        fetch(&db, "p2", "C");
        use crate::services::model_fetch::FetchedModel;
        db.replace_fetched_models(
            APP,
            "p3",
            &[
                FetchedModel { id: "C".into(), owned_by: None },
                FetchedModel { id: "D".into(), owned_by: None },
            ],
            100,
        )
        .unwrap();
        let cid = db.create_custom_aggregate(APP, "CCC", &["C".into(), "D".into()]).unwrap();

        // 删 p3：C 只剩 p2，D 归零 → missing_members 含 D。
        db.remove_from_failover_queue(APP, "p3").unwrap();
        let views = build_custom_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 1);
        let v = &views[0];
        assert!(!v.is_empty);
        assert_eq!(v.members.len(), 1);
        assert_eq!(v.members[0].provider_id, "p2");
        assert_eq!(v.members[0].model_id, "C");
        assert_eq!(v.missing_members, vec!["D"]);

        // 再删 p2：全部成员归零 → is_empty=true，但定义仍在（绝不自动删）。
        db.remove_from_failover_queue(APP, "p2").unwrap();
        let views = build_custom_aggregates(&db, APP).unwrap();
        assert_eq!(views.len(), 1, "定义仍在");
        assert!(views[0].is_empty);
        assert!(views[0].members.is_empty());
        assert_eq!(views[0].missing_members, vec!["C", "D"]);
        // 底层定义确实未被删除。
        assert_eq!(db.list_custom_aggregates(APP).unwrap().len(), 1);
        assert_eq!(db.list_custom_aggregates(APP).unwrap()[0].id, cid);
    }

    // R17：展平去重——同一 (provider, model) 在多成员间重复出现只保留一次。
    #[test]
    fn r17_flatten_dedup() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        fetch(&db, "p1", "C");
        // 自定义聚合成员里重复列同一个 key C。
        let id = db.create_custom_aggregate(APP, "dup", &["C".into(), "C".into()]).unwrap();
        let flat = flatten_aggregate_ref(&db, APP, &AggregateRef::Custom(id)).unwrap();
        assert_eq!(flat.len(), 1, "重复 (p1, C) 应去重");
    }

    // flatten 自动聚合（单成员特例）。
    #[test]
    fn flatten_auto_ref() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        insert_provider(&db, "p2", true, 1);
        fetch(&db, "p1", "C");
        fetch(&db, "p2", "C");

        let flat = flatten_aggregate_ref(&db, APP, &AggregateRef::Auto("C".into())).unwrap();
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].provider_id, "p1");
        assert_eq!(flat[1].provider_id, "p2");
    }

    // 未知 ref → 空 Vec（不报错，C3 决定降级）。
    #[test]
    fn flatten_unknown_ref_returns_empty() {
        let db = Database::memory().unwrap();
        insert_provider(&db, "p1", true, 0);
        fetch(&db, "p1", "C");

        assert!(flatten_aggregate_ref(&db, APP, &AggregateRef::Auto("nope".into()))
            .unwrap()
            .is_empty());
        assert!(flatten_aggregate_ref(&db, APP, &AggregateRef::Custom("missing".into()))
            .unwrap()
            .is_empty());
    }
}
