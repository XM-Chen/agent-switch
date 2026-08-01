use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{params, OptionalExtension};

pub const GATEWAY_CONFIG_ID: i64 = 1;
pub const LEGACY_GATEWAY_TOKEN_SETTING_KEY: &str = "claude_desktop_gateway_token";

#[derive(Debug, Clone)]
pub struct GatewayAuthConfig {
    pub auth_required: bool,
}

#[derive(Debug, Clone)]
pub struct GatewayApiKeyRecord {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

impl Database {
    pub fn get_gateway_auth_config(&self) -> Result<GatewayAuthConfig, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT auth_required FROM gateway_config WHERE id = ?1",
            [GATEWAY_CONFIG_ID],
            |row| {
                Ok(GatewayAuthConfig {
                    auth_required: row.get::<_, i64>(0)? != 0,
                })
            },
        )
        .map_err(|e| AppError::Database(format!("读取网关鉴权配置失败: {e}")))
    }

    pub fn set_gateway_auth_required(&self, required: bool) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "UPDATE gateway_config SET auth_required = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                i64::from(required),
                chrono::Utc::now().timestamp_millis(),
                GATEWAY_CONFIG_ID
            ],
        )
        .map_err(|e| AppError::Database(format!("更新网关鉴权配置失败: {e}")))?;
        Ok(())
    }

    pub fn list_gateway_api_keys(&self) -> Result<Vec<GatewayApiKeyRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare(
                "SELECT id, name, key_hash, key_prefix, created_at, revoked_at, last_used_at
                 FROM gateway_api_keys ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(format!("准备读取网关 API Key 失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GatewayApiKeyRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    key_hash: row.get(2)?,
                    key_prefix: row.get(3)?,
                    created_at: row.get(4)?,
                    revoked_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                })
            })
            .map_err(|e| AppError::Database(format!("读取网关 API Key 失败: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("解析网关 API Key 失败: {e}")))
    }

    pub fn get_gateway_api_key(
        &self,
        key_id: &str,
    ) -> Result<Option<GatewayApiKeyRecord>, AppError> {
        let conn = lock_conn!(self.conn);
        conn.query_row(
            "SELECT id, name, key_hash, key_prefix, created_at, revoked_at, last_used_at
             FROM gateway_api_keys WHERE id = ?1",
            [key_id],
            |row| {
                Ok(GatewayApiKeyRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    key_hash: row.get(2)?,
                    key_prefix: row.get(3)?,
                    created_at: row.get(4)?,
                    revoked_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| AppError::Database(format!("读取网关 API Key 失败: {e}")))
    }

    pub fn insert_gateway_api_key(&self, record: &GatewayApiKeyRecord) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT INTO gateway_api_keys
             (id, name, key_hash, key_prefix, created_at, revoked_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id,
                record.name,
                record.key_hash,
                record.key_prefix,
                record.created_at,
                record.revoked_at,
                record.last_used_at,
            ],
        )
        .map_err(|e| AppError::Database(format!("保存网关 API Key 失败: {e}")))?;
        Ok(())
    }

    pub fn revoke_gateway_api_key(&self, key_id: &str, revoked_at: i64) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let updated = conn
            .execute(
                "UPDATE gateway_api_keys SET revoked_at = ?1
                 WHERE id = ?2 AND revoked_at IS NULL",
                params![revoked_at, key_id],
            )
            .map_err(|e| AppError::Database(format!("撤销网关 API Key 失败: {e}")))?;
        Ok(updated > 0)
    }

    pub fn touch_gateway_api_key(&self, key_id: &str, used_at: i64) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "UPDATE gateway_api_keys SET last_used_at = ?1 WHERE id = ?2",
            params![used_at, key_id],
        )
        .map_err(|e| AppError::Database(format!("更新网关 API Key 使用时间失败: {e}")))?;
        Ok(())
    }

    pub fn take_legacy_gateway_token(&self) -> Result<Option<String>, AppError> {
        let mut conn = lock_conn!(self.conn);
        let tx = conn
            .transaction()
            .map_err(|e| AppError::Database(format!("开始迁移旧网关 token 失败: {e}")))?;
        let token = tx
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [LEGACY_GATEWAY_TOKEN_SETTING_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| AppError::Database(format!("读取旧网关 token 失败: {e}")))?;
        if token.is_some() {
            tx.execute(
                "DELETE FROM settings WHERE key = ?1",
                [LEGACY_GATEWAY_TOKEN_SETTING_KEY],
            )
            .map_err(|e| AppError::Database(format!("删除旧网关 token 失败: {e}")))?;
        }
        tx.commit()
            .map_err(|e| AppError::Database(format!("提交旧网关 token 迁移失败: {e}")))?;
        Ok(token.filter(|value| !value.trim().is_empty()))
    }
}
