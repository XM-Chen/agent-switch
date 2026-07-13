//! Provider 模型缓存 DAO
//!
//! 管理 `provider_models` 表：每个上游 fetch 到的模型列表 + 用户手动补录的模型。
//! 是 CC 聚合功能（C2 派生 / C3 路由 / C4 UI）的底层数据源。
//!
//! 覆盖规则（父任务 D6）：
//! - `source='fetched'`：由 `/v1/models` 拉取；一次刷新整体覆盖同上游的 fetched 行；
//! - `source='manual'`：用户手动补录；不受任何自动刷新增删影响，永久保留直到手动删除；
//! - 唯一键 `(provider_id, app_type, model_id)`：手动 id 与 fetch id 撞键时保留 manual 标记。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::model_fetch::FetchedModel;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// 模型来源：`/v1/models` 拉取。
pub const MODEL_SOURCE_FETCHED: &str = "fetched";
/// 模型来源：用户手动补录。
pub const MODEL_SOURCE_MANUAL: &str = "manual";

/// 缓存的上游模型行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    pub provider_id: String,
    pub app_type: String,
    /// 上游返回的 model id 原文，不做归一化改写。
    pub model_id: String,
    /// `fetched` | `manual`。
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    /// 毫秒 epoch，与 providers.created_at 口径一致。
    pub fetched_at: i64,
}

impl Database {
    /// 列出模型缓存。`provider_id` 为 `None` 时返回该 app_type 全部行。
    pub fn list_provider_models(
        &self,
        app_type: &str,
        provider_id: Option<&str>,
    ) -> Result<Vec<ProviderModel>, AppError> {
        let conn = lock_conn!(self.conn);

        // 稳定排序：便于测试与 UI 展示一致。
        let (sql, bind_provider) = match provider_id {
            Some(_) => (
                "SELECT provider_id, app_type, model_id, source, owned_by, fetched_at
                 FROM provider_models
                 WHERE app_type = ?1 AND provider_id = ?2
                 ORDER BY provider_id ASC, model_id ASC",
                true,
            ),
            None => (
                "SELECT provider_id, app_type, model_id, source, owned_by, fetched_at
                 FROM provider_models
                 WHERE app_type = ?1
                 ORDER BY provider_id ASC, model_id ASC",
                false,
            ),
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| AppError::Database(e.to_string()))?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<ProviderModel> {
            Ok(ProviderModel {
                provider_id: row.get(0)?,
                app_type: row.get(1)?,
                model_id: row.get(2)?,
                source: row.get(3)?,
                owned_by: row.get(4)?,
                fetched_at: row.get(5)?,
            })
        };

        let rows = if bind_provider {
            stmt.query_map(params![app_type, provider_id.unwrap_or("")], map_row)
        } else {
            stmt.query_map(params![app_type], map_row)
        }
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(rows)
    }

    /// 用一次 fetch 结果整体覆盖某上游的 `source='fetched'` 行。
    ///
    /// 事务内：先删该上游全部 fetched 行，再插入新 fetched 行。`source='manual'`
    /// 行绝不因此被删除（R8）；若某 model_id 已存在 manual 行，则跳过该 id 的
    /// fetched 插入以保留 manual 标记（R12）。
    pub fn replace_fetched_models(
        &self,
        app_type: &str,
        provider_id: &str,
        models: &[FetchedModel],
        fetched_at: i64,
    ) -> Result<(), AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM provider_models
             WHERE app_type = ?1 AND provider_id = ?2 AND source = ?3",
            params![app_type, provider_id, MODEL_SOURCE_FETCHED],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

        for model in models {
            // manual 行与 fetch 撞键时保留 manual：INSERT OR IGNORE 不会覆盖已存在的
            // 主键（此时残留的必是 manual 行，fetched 行上面已清空）。
            tx.execute(
                "INSERT OR IGNORE INTO provider_models
                    (provider_id, app_type, model_id, source, owned_by, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    provider_id,
                    app_type,
                    model.id,
                    MODEL_SOURCE_FETCHED,
                    model.owned_by,
                    fetched_at
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }

        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 手动补录一条模型 id（`source='manual'`）。
    ///
    /// `INSERT OR REPLACE`：若已存在 fetched 同行，升级为 manual 并保留（D6）。
    pub fn upsert_manual_model(
        &self,
        app_type: &str,
        provider_id: &str,
        model_id: &str,
        fetched_at: i64,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT OR REPLACE INTO provider_models
                (provider_id, app_type, model_id, source, owned_by, fetched_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            params![
                provider_id,
                app_type,
                model_id,
                MODEL_SOURCE_MANUAL,
                fetched_at
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 删除一条手动补录的模型。仅删 `source='manual'` 的指定行，不动 fetched 行。
    pub fn delete_manual_model(
        &self,
        app_type: &str,
        provider_id: &str,
        model_id: &str,
    ) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "DELETE FROM provider_models
             WHERE app_type = ?1 AND provider_id = ?2 AND model_id = ?3 AND source = ?4",
            params![app_type, provider_id, model_id, MODEL_SOURCE_MANUAL],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 该上游是否存在任何缓存行（fetched 或 manual）。防抖触发判定用（R14）。
    pub fn has_any_cached_models(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM provider_models
                 WHERE app_type = ?1 AND provider_id = ?2",
                params![app_type, provider_id],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use rusqlite::params;

    const APP: &str = "claude";

    fn insert_provider(db: &Database, id: &str) {
        let conn = db.conn.lock().expect("lock conn");
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta, is_current)
             VALUES (?1, ?2, ?3, '{}', '{}', 0)",
            params![id, APP, id],
        )
        .expect("insert provider");
    }

    fn fetched(id: &str) -> FetchedModel {
        FetchedModel {
            id: id.to_string(),
            owned_by: Some("upstream".to_string()),
        }
    }

    #[test]
    fn replace_fetched_overwrites_only_fetched_rows() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        db.replace_fetched_models(APP, "p1", &[fetched("a"), fetched("b")], 100)
            .expect("first fetch");
        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.source == MODEL_SOURCE_FETCHED));

        // 第二次 fetch 只返回 c：a/b 应被覆盖删除，只剩 c。
        db.replace_fetched_models(APP, "p1", &[fetched("c")], 200)
            .expect("second fetch");
        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "c");
        assert_eq!(rows[0].fetched_at, 200);
    }

    #[test]
    fn manual_rows_survive_fetch_refresh() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        db.upsert_manual_model(APP, "p1", "manual-x", 50)
            .expect("add manual");
        db.replace_fetched_models(APP, "p1", &[fetched("a")], 100)
            .expect("fetch");

        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 2);
        let manual = rows
            .iter()
            .find(|r| r.model_id == "manual-x")
            .expect("manual row present");
        assert_eq!(manual.source, MODEL_SOURCE_MANUAL);

        // 再次 fetch 完全不同的集合，manual 行仍然存在。
        db.replace_fetched_models(APP, "p1", &[fetched("z")], 300)
            .expect("fetch again");
        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert!(rows.iter().any(|r| r.model_id == "manual-x"));
    }

    #[test]
    fn manual_wins_when_fetch_returns_same_id() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        db.upsert_manual_model(APP, "p1", "glm-4.6", 50)
            .expect("add manual");
        // fetch 返回同名 id：应保留 manual 标记，不被降级为 fetched。
        db.replace_fetched_models(APP, "p1", &[fetched("glm-4.6")], 100)
            .expect("fetch");

        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "glm-4.6");
        assert_eq!(rows[0].source, MODEL_SOURCE_MANUAL);
    }

    #[test]
    fn upsert_manual_upgrades_existing_fetched_row() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        db.replace_fetched_models(APP, "p1", &[fetched("shared")], 100)
            .expect("fetch");
        db.upsert_manual_model(APP, "p1", "shared", 200)
            .expect("upsert manual over fetched");

        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, MODEL_SOURCE_MANUAL);
    }

    #[test]
    fn delete_manual_only_removes_manual_row() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        db.replace_fetched_models(APP, "p1", &[fetched("keep")], 100)
            .expect("fetch");
        db.upsert_manual_model(APP, "p1", "drop", 100)
            .expect("add manual");

        db.delete_manual_model(APP, "p1", "drop")
            .expect("delete manual");
        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "keep");

        // 删除 fetched id 走 manual 删除接口应无效（不误删 fetched）。
        db.delete_manual_model(APP, "p1", "keep")
            .expect("delete non-manual is no-op");
        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn deleting_provider_cascades_cached_models() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");
        db.replace_fetched_models(APP, "p1", &[fetched("a")], 100)
            .expect("fetch");
        db.upsert_manual_model(APP, "p1", "m", 100).expect("manual");

        {
            let conn = db.conn.lock().expect("lock conn");
            conn.execute(
                "DELETE FROM providers WHERE id = ?1 AND app_type = ?2",
                params!["p1", APP],
            )
            .expect("delete provider");
        }

        let rows = db.list_provider_models(APP, Some("p1")).expect("list");
        assert!(rows.is_empty(), "CASCADE should clear cached models");
    }

    #[test]
    fn has_any_cached_reflects_presence() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");

        assert!(!db.has_any_cached_models(APP, "p1").expect("check empty"));
        db.replace_fetched_models(APP, "p1", &[fetched("a")], 100)
            .expect("fetch");
        assert!(db.has_any_cached_models(APP, "p1").expect("check present"));
    }

    #[test]
    fn list_all_providers_when_id_omitted() {
        let db = Database::memory().expect("memory db");
        insert_provider(&db, "p1");
        insert_provider(&db, "p2");
        db.replace_fetched_models(APP, "p1", &[fetched("a")], 100)
            .expect("fetch p1");
        db.replace_fetched_models(APP, "p2", &[fetched("b")], 100)
            .expect("fetch p2");

        let all = db.list_provider_models(APP, None).expect("list all");
        assert_eq!(all.len(), 2);
    }
}
