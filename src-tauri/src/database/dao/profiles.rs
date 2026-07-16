//! 项目预设（Project Profiles）数据访问对象
//!
//! profiles 表存储项目预设记录（id/name/payload/排序/时间戳）。
//! 每个 scope（claude / claude-desktop / codex）的「当前预设」指针
//! **不进 profiles 表**，而是存于 settings 表，键为 `current_profile_id_<scope>`。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::params;

/// 当前预设指针的 settings 键前缀（按 scope 区分）。
const CURRENT_PROFILE_ID_KEY_PREFIX: &str = "current_profile_id_";

/// 构造某 scope 的当前预设指针键。
fn current_profile_key(scope: &str) -> String {
    format!("{CURRENT_PROFILE_ID_KEY_PREFIX}{scope}")
}

/// 项目预设记录（DAO 层原始行）。
///
/// `payload` 为 `ProfilePayload` 序列化后的原始 JSON 文本，由 service 层解析；
/// DAO 层不感知其结构，只做整行存取。
#[derive(Debug, Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub payload: String,
    pub sort_order: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

impl Database {
    /// 列出全部预设。
    ///
    /// 排序：先按 `sort_order`（NULL 排最后），再按 `created_at`、`id` 稳定兜底。
    pub fn get_all_profiles(&self) -> Result<Vec<Profile>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, payload, sort_order, created_at, updated_at \
                 FROM profiles \
                 ORDER BY sort_order IS NULL, sort_order, created_at, id",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Profile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    payload: row.get(2)?,
                    sort_order: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(profiles)
    }

    /// 按 id 获取单个预设，不存在返回 `None`。
    pub fn get_profile(&self, id: &str) -> Result<Option<Profile>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, payload, sort_order, created_at, updated_at \
                 FROM profiles WHERE id = ?1",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut rows = stmt
            .query(params![id])
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            Ok(Some(Profile {
                id: row.get(0).map_err(|e| AppError::Database(e.to_string()))?,
                name: row.get(1).map_err(|e| AppError::Database(e.to_string()))?,
                payload: row.get(2).map_err(|e| AppError::Database(e.to_string()))?,
                sort_order: row.get(3).map_err(|e| AppError::Database(e.to_string()))?,
                created_at: row.get(4).map_err(|e| AppError::Database(e.to_string()))?,
                updated_at: row.get(5).map_err(|e| AppError::Database(e.to_string()))?,
            }))
        } else {
            Ok(None)
        }
    }

    /// 保存（新建或整行替换）一个预设。
    pub fn save_profile(&self, profile: &Profile) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT OR REPLACE INTO profiles \
             (id, name, payload, sort_order, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                profile.id,
                profile.name,
                profile.payload,
                profile.sort_order,
                profile.created_at,
                profile.updated_at,
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// 删除一个预设，返回是否实际删除了行。
    pub fn delete_profile(&self, id: &str) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let affected = conn
            .execute("DELETE FROM profiles WHERE id = ?1", params![id])
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(affected > 0)
    }

    /// 读取某 scope 的当前预设指针。
    pub fn get_current_profile_id(&self, scope: &str) -> Result<Option<String>, AppError> {
        self.get_setting(&current_profile_key(scope))
    }

    /// 设置某 scope 的当前预设指针；`None` 清除该指针。
    pub fn set_current_profile_id(&self, scope: &str, id: Option<&str>) -> Result<(), AppError> {
        let key = current_profile_key(scope);
        match id {
            Some(value) => self.set_setting(&key, value),
            None => {
                let conn = lock_conn!(self.conn);
                conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile(id: &str, name: &str) -> Profile {
        Profile {
            id: id.to_string(),
            name: name.to_string(),
            payload: "{}".to_string(),
            sort_order: None,
            created_at: Some(1),
            updated_at: Some(1),
        }
    }

    #[test]
    fn test_profile_crud_roundtrip() {
        let db = Database::memory().unwrap();

        // 空表
        assert!(db.get_all_profiles().unwrap().is_empty());
        assert!(db.get_profile("p1").unwrap().is_none());

        // 新建
        db.save_profile(&sample_profile("p1", "Alpha")).unwrap();
        db.save_profile(&sample_profile("p2", "Beta")).unwrap();

        let all = db.get_all_profiles().unwrap();
        assert_eq!(all.len(), 2);

        let fetched = db.get_profile("p1").unwrap().unwrap();
        assert_eq!(fetched.name, "Alpha");

        // 替换（同 id）
        let mut updated = sample_profile("p1", "Alpha2");
        updated.payload = r#"{"x":1}"#.to_string();
        db.save_profile(&updated).unwrap();
        let fetched = db.get_profile("p1").unwrap().unwrap();
        assert_eq!(fetched.name, "Alpha2");
        assert_eq!(fetched.payload, r#"{"x":1}"#);
        assert_eq!(db.get_all_profiles().unwrap().len(), 2);

        // 删除
        assert!(db.delete_profile("p1").unwrap());
        assert!(!db.delete_profile("p1").unwrap());
        assert!(db.get_profile("p1").unwrap().is_none());
        assert_eq!(db.get_all_profiles().unwrap().len(), 1);
    }

    #[test]
    fn test_current_profile_id_is_scoped() {
        let db = Database::memory().unwrap();

        // 初始为空
        assert!(db.get_current_profile_id("claude").unwrap().is_none());

        // 各 scope 独立
        db.set_current_profile_id("claude", Some("p-claude"))
            .unwrap();
        db.set_current_profile_id("codex", Some("p-codex")).unwrap();

        assert_eq!(
            db.get_current_profile_id("claude").unwrap().as_deref(),
            Some("p-claude")
        );
        assert_eq!(
            db.get_current_profile_id("codex").unwrap().as_deref(),
            Some("p-codex")
        );
        assert!(db
            .get_current_profile_id("claude-desktop")
            .unwrap()
            .is_none());

        // 清除只影响目标 scope
        db.set_current_profile_id("claude", None).unwrap();
        assert!(db.get_current_profile_id("claude").unwrap().is_none());
        assert_eq!(
            db.get_current_profile_id("codex").unwrap().as_deref(),
            Some("p-codex")
        );
    }
}
