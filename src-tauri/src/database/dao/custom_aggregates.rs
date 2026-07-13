//! 自定义聚合 DAO（CC 聚合功能 C2）
//!
//! 管理 `custom_aggregates` 表：用户持久化的自定义聚合**定义**（名称 + 有序成员）。
//! 只存定义，不存内容——内容由 `services/aggregate.rs` 现算派生（D1/D7）。
//!
//! 成员列表 `ordered_members` 存的是**自动聚合的 key（= 模型 id 原文）**，
//! 非上游、非具体模型行（R9）。定义不受自动刷新增删影响（R11）；全部成员归零
//! 时也**只标记不自动删**（R13），删除仅由用户显式触发。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// 自定义聚合原始行（区别于派生的 `CustomAggregateView`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CustomAggregate {
    pub id: String,
    pub app_type: String,
    pub name: String,
    /// 有序成员列表，元素 = 自动聚合 key（模型 id 原文）。
    pub ordered_members: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_index: Option<i64>,
}

/// 聚合引用消歧（D7 统一路由抽象）。
///
/// `tierSelection` 的值既可指自动聚合又可指自定义聚合。用带标签枚举序列化，
/// 避免自动聚合 key 与自定义聚合 id 撞名：
/// `{ "type": "auto", "value": "glm-4.6" }` 或 `{ "type": "custom", "value": "<uuid>" }`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum AggregateRef {
    /// 自动聚合 key（模型 id 原文）。
    Auto(String),
    /// custom_aggregates.id。
    Custom(String),
}

/// tier → 聚合选择（D10）。tier 名称集合沿用 `ModelMapping` 的 5 档。
///
/// 每个 tier 的值可为 `None`（未设置）或指向一个自动/自定义聚合。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TierSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sonnet: Option<AggregateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opus: Option<AggregateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haiku: Option<AggregateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fable: Option<AggregateRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<AggregateRef>,
}

/// CC 聚合模式配置（D10）。存于 settings 表，key = `cc_aggregate_config:{app_type}`。
///
/// `Default` 为 `enabled=false`、全 tier 未设 → 对 C3 而言聚合模式默认关，行为不变。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CcAggregateConfig {
    /// 聚合模式总开关（默认关闭）。
    #[serde(default)]
    pub enabled: bool,
    /// tier → 聚合选择。
    #[serde(default)]
    pub tier_selection: TierSelection,
}

impl Database {
    /// 列出某 app_type 的全部自定义聚合定义，按 `sort_index` 稳定排序。
    pub fn list_custom_aggregates(
        &self,
        app_type: &str,
    ) -> Result<Vec<CustomAggregate>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, app_type, name, ordered_members, sort_index
                 FROM custom_aggregates
                 WHERE app_type = ?1
                 ORDER BY COALESCE(sort_index, 999999), id ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![app_type], |row| {
                let members_str: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    members_str,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut result = Vec::with_capacity(rows.len());
        for (id, app_type, name, members_str, sort_index) in rows {
            let ordered_members: Vec<String> =
                serde_json::from_str(&members_str).unwrap_or_default();
            result.push(CustomAggregate {
                id,
                app_type,
                name,
                ordered_members,
                sort_index,
            });
        }
        Ok(result)
    }

    /// 创建一个自定义聚合，返回新生成的 id。
    ///
    /// `sort_index` 取当前 app_type 下最大 sort_index + 1（追加到末尾）。
    pub fn create_custom_aggregate(
        &self,
        app_type: &str,
        name: &str,
        members: &[String],
    ) -> Result<String, AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let members_json = serde_json::to_string(members)
            .map_err(|e| AppError::Database(format!("序列化自定义聚合成员失败: {e}")))?;
        let now = chrono::Utc::now().timestamp_millis();

        let conn = lock_conn!(self.conn);
        // 追加到末尾：取现有最大 sort_index + 1，空表则为 0。
        let next_sort: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index), -1) + 1 FROM custom_aggregates WHERE app_type = ?1",
                params![app_type],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        conn.execute(
            "INSERT INTO custom_aggregates
                (id, app_type, name, ordered_members, sort_index, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, app_type, name, members_json, next_sort, now],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(id)
    }

    /// 更新自定义聚合的名称和/或成员列表。`None` 参数表示不改动该字段。
    pub fn update_custom_aggregate(
        &self,
        id: &str,
        name: Option<&str>,
        members: Option<&[String]>,
    ) -> Result<(), AppError> {
        // 无字段需要更新时直接返回，避免生成无意义的 SQL。
        if name.is_none() && members.is_none() {
            return Ok(());
        }

        let now = chrono::Utc::now().timestamp_millis();
        let conn = lock_conn!(self.conn);

        if let Some(name) = name {
            conn.execute(
                "UPDATE custom_aggregates SET name = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, name, now],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        if let Some(members) = members {
            let members_json = serde_json::to_string(members)
                .map_err(|e| AppError::Database(format!("序列化自定义聚合成员失败: {e}")))?;
            conn.execute(
                "UPDATE custom_aggregates SET ordered_members = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, members_json, now],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }

    /// 删除一个自定义聚合（用户显式删除，非归零自动删除）。
    pub fn delete_custom_aggregate(&self, id: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM custom_aggregates WHERE id = ?1",
            params![id],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 按给定顺序重排某 app_type 下的自定义聚合（拖拽排序）。
    ///
    /// 未出现在 `ordered_ids` 中的行 `sort_index` 保持不变。
    pub fn reorder_custom_aggregates(
        &self,
        app_type: &str,
        ordered_ids: &[String],
    ) -> Result<(), AppError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;
        for (idx, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE custom_aggregates SET sort_index = ?2, updated_at = ?3
                 WHERE id = ?1 AND app_type = ?4",
                params![id, idx as i64, now, app_type],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 读取 CC 聚合模式配置（D10）。空缺 → `Default`（`enabled=false`、全 tier 未设）。
    ///
    /// key 带 app 维度：`cc_aggregate_config:{app_type}`（当前仅 claude，键名预留
    /// app 维度避免将来扩展改键）。
    pub fn get_cc_aggregate_config(&self, app_type: &str) -> Result<CcAggregateConfig, AppError> {
        let key = format!("cc_aggregate_config:{app_type}");
        match self.get_setting(&key)? {
            Some(json) => serde_json::from_str(&json)
                .map_err(|e| AppError::Database(format!("解析 CC 聚合配置失败: {e}"))),
            None => Ok(CcAggregateConfig::default()),
        }
    }

    /// 写入 CC 聚合模式配置（D10）。
    pub fn set_cc_aggregate_config(
        &self,
        app_type: &str,
        config: &CcAggregateConfig,
    ) -> Result<(), AppError> {
        let key = format!("cc_aggregate_config:{app_type}");
        let json = serde_json::to_string(config)
            .map_err(|e| AppError::Database(format!("序列化 CC 聚合配置失败: {e}")))?;
        self.set_setting(&key, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "claude";

    #[test]
    fn create_and_list_roundtrip() {
        let db = Database::memory().expect("memory db");
        let id = db
            .create_custom_aggregate(APP, "CCC", &["glm-4.6".into(), "gpt-5".into()])
            .expect("create");

        let all = db.list_custom_aggregates(APP).expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].name, "CCC");
        // JSON 数组保序。
        assert_eq!(all[0].ordered_members, vec!["glm-4.6", "gpt-5"]);
    }

    #[test]
    fn update_name_and_members() {
        let db = Database::memory().expect("memory db");
        let id = db
            .create_custom_aggregate(APP, "old", &["a".into()])
            .expect("create");

        db.update_custom_aggregate(&id, Some("new"), None)
            .expect("update name");
        let all = db.list_custom_aggregates(APP).expect("list");
        assert_eq!(all[0].name, "new");
        assert_eq!(all[0].ordered_members, vec!["a"]);

        db.update_custom_aggregate(&id, None, Some(&["x".into(), "y".into()]))
            .expect("update members");
        let all = db.list_custom_aggregates(APP).expect("list");
        assert_eq!(all[0].name, "new");
        assert_eq!(all[0].ordered_members, vec!["x", "y"]);

        // 双 None 是 no-op，不报错、不改动。
        db.update_custom_aggregate(&id, None, None).expect("noop");
        let all = db.list_custom_aggregates(APP).expect("list");
        assert_eq!(all[0].name, "new");
    }

    #[test]
    fn delete_removes_definition() {
        let db = Database::memory().expect("memory db");
        let id = db
            .create_custom_aggregate(APP, "gone", &[])
            .expect("create");
        db.delete_custom_aggregate(&id).expect("delete");
        assert!(db.list_custom_aggregates(APP).expect("list").is_empty());
    }

    #[test]
    fn reorder_changes_list_order() {
        let db = Database::memory().expect("memory db");
        let a = db.create_custom_aggregate(APP, "a", &[]).expect("a");
        let b = db.create_custom_aggregate(APP, "b", &[]).expect("b");
        let c = db.create_custom_aggregate(APP, "c", &[]).expect("c");

        // 初始顺序 a, b, c（按插入递增的 sort_index）。
        let ids: Vec<String> = db
            .list_custom_aggregates(APP)
            .expect("list")
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec![a.clone(), b.clone(), c.clone()]);

        // 重排为 c, a, b。
        db.reorder_custom_aggregates(APP, &[c.clone(), a.clone(), b.clone()])
            .expect("reorder");
        let ids: Vec<String> = db
            .list_custom_aggregates(APP)
            .expect("list")
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(ids, vec![c, a, b]);
    }

    #[test]
    fn create_appends_to_end() {
        let db = Database::memory().expect("memory db");
        db.create_custom_aggregate(APP, "first", &[]).expect("1");
        db.create_custom_aggregate(APP, "second", &[]).expect("2");
        let all = db.list_custom_aggregates(APP).expect("list");
        assert_eq!(all[0].name, "first");
        assert_eq!(all[1].name, "second");
        assert_eq!(all[0].sort_index, Some(0));
        assert_eq!(all[1].sort_index, Some(1));
    }

    #[test]
    fn cc_aggregate_config_defaults_to_disabled() {
        let db = Database::memory().expect("memory db");
        let cfg = db.get_cc_aggregate_config(APP).expect("get default");
        assert!(!cfg.enabled);
        assert!(cfg.tier_selection.sonnet.is_none());
        assert!(cfg.tier_selection.opus.is_none());
        assert!(cfg.tier_selection.haiku.is_none());
        assert!(cfg.tier_selection.fable.is_none());
        assert!(cfg.tier_selection.default.is_none());
    }

    #[test]
    fn cc_aggregate_config_roundtrip() {
        let db = Database::memory().expect("memory db");
        let cfg = CcAggregateConfig {
            enabled: true,
            tier_selection: TierSelection {
                sonnet: Some(AggregateRef::Auto("glm-4.6".into())),
                opus: Some(AggregateRef::Custom("cust-1".into())),
                ..Default::default()
            },
        };
        db.set_cc_aggregate_config(APP, &cfg).expect("set");
        let got = db.get_cc_aggregate_config(APP).expect("get");
        assert_eq!(got, cfg);
        // AggregateRef 带标签枚举正确回读。
        assert_eq!(
            got.tier_selection.sonnet,
            Some(AggregateRef::Auto("glm-4.6".into()))
        );
        assert_eq!(
            got.tier_selection.opus,
            Some(AggregateRef::Custom("cust-1".into()))
        );
    }
}
