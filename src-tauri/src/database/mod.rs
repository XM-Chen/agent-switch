//! 数据库模块 - SQLite 数据持久化
//!
//! 此模块提供应用的核心数据存储功能，包括：
//! - 供应商配置管理
//! - MCP 服务器配置
//! - 提示词管理
//! - Skills 管理
//! - 通用设置存储
//!
//! ## 架构设计
//!
//! ```text
//! database/
//! ├── mod.rs        - Database 结构体 + 初始化
//! ├── schema.rs     - 表结构定义 + Schema 迁移
//! ├── backup.rs     - SQL 导入导出 + 快照备份
//! ├── migration.rs  - JSON → SQLite 数据迁移
//! └── dao/          - 数据访问对象
//!     ├── providers.rs
//!     ├── mcp.rs
//!     ├── prompts.rs
//!     ├── skills.rs
//!     └── settings.rs
//! ```

pub(crate) mod backup;
mod dao;
mod gateway_migration;
mod migration;
mod schema;

#[cfg(test)]
mod tests;

// DAO 类型导出供外部使用
pub(crate) use dao::providers_seed::{
    is_official_seed_id, CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID, CODEX_OFFICIAL_PROVIDER_ID,
};
pub(crate) use dao::proxy::{
    validate_cost_multiplier, validate_pricing_source, PRICING_SOURCE_REQUEST,
    PRICING_SOURCE_RESPONSE,
};
pub use dao::FailoverQueueItem;
pub use dao::GatewayApiKeyRecord;
pub use dao::{
    CreateGatewayModelInput, CreateGatewayUpstreamInput, CreateRouteTargetInput,
    GatewayConfigRecord, GatewayMigrationIssue, GatewayModelRecord, GatewayUpstreamDto,
    ModelAliasRecord, RouteTargetHealthRecord, RouteTargetRecord, UpdateGatewayModelInput,
    UpdateGatewayUpstreamInput, UpdateRouteTargetInput, UpstreamCredentialHintDto,
    UpstreamModelRecord, UpstreamRecord,
};
pub use dao::{Profile, ProviderModel};
// CustomAggregate / TierSelection 目前仅在 dao 内部命名，但作为 C2 对 C3/C4 的
// 契约类型对外导出（dao 模块本身私有，此处是唯一 crate 级暴露点）。
#[allow(unused_imports)]
pub use dao::{AggregateRef, CcAggregateConfig, CustomAggregate, TierSelection};

use crate::config::get_app_config_dir;
use crate::error::AppError;
use crate::gateway::credential;
use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
use rusqlite::{hooks::Action, Connection};
use serde::Serialize;
use std::sync::Mutex;

// DAO 方法通过 impl Database 提供，无需额外导出

/// 当前 Schema 版本号
///
/// v18 净化迁移由三道 fail-closed 门保护：早期 v17 泛型凭据精确重分类、启用路由
/// credential readiness 审计，以及 scope/hash/row-count/FK/DPAPI 均通过的本机回滚恢复点。
pub(crate) const SCHEMA_VERSION: i32 = 18;

/// 安全地序列化 JSON，避免 unwrap panic
pub(crate) fn to_json_string<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|e| AppError::Config(format!("JSON serialization failed: {e}")))
}

/// 安全地获取 Mutex 锁，避免 unwrap panic
macro_rules! lock_conn {
    ($mutex:expr) => {
        $mutex
            .lock()
            .map_err(|e| AppError::Database(format!("Mutex lock failed: {}", e)))?
    };
}

// 导出宏供子模块使用
pub(crate) use lock_conn;

/// 数据库连接封装
///
/// 使用 Mutex 包装 Connection 以支持在多线程环境（如 Tauri State）中共享。
/// rusqlite::Connection 本身不是 Sync 的，因此需要这层包装。
pub struct Database {
    pub(crate) conn: Mutex<Connection>,
}

fn register_db_change_hook(conn: &Connection) {
    conn.update_hook(Some(
        |action: Action, _database: &str, table: &str, _row_id: i64| match action {
            Action::SQLITE_INSERT | Action::SQLITE_UPDATE | Action::SQLITE_DELETE => {
                crate::services::webdav_auto_sync::notify_db_changed(table);
                crate::services::s3_auto_sync::notify_db_changed(table);
            }
            _ => {}
        },
    ));
}

impl Database {
    /// v18 净化前的 fail-closed 凭据就绪门。
    ///
    /// 审计所有启用上游（包括尚未挂路由的上游）：每个上游必须恰有一个精确类型化凭据，
    /// 使用当前平台 protector 可解密，payload 与 adapter 语义均可直接运行。此过程只读
    /// Agent Switch 自有数据库，不探测或读取任何客户端配置。
    pub(crate) fn ensure_v18_credential_readiness(&self) -> Result<(), AppError> {
        let protector = PlatformCredentialProtector;
        self.ensure_v18_credential_readiness_with_protector(&protector)
    }

    fn ensure_v18_credential_readiness_with_protector(
        &self,
        protector: &dyn CredentialProtector,
    ) -> Result<(), AppError> {
        #[derive(Debug)]
        struct Candidate {
            upstream_id: String,
            protocol: String,
            adapter_type: String,
            credential_kind: String,
            encrypted_payload: Vec<u8>,
            encryption_scheme: String,
        }

        let candidates = {
            let conn = lock_conn!(self.conn);
            let mut stmt = conn
                .prepare(
                    "SELECT u.id, u.protocol, u.adapter_type,
                            COUNT(c.id) AS credential_count,
                            MIN(c.credential_kind), MIN(c.encrypted_payload),
                            MIN(c.encryption_scheme)
                     FROM upstreams u
                     LEFT JOIN upstream_credentials c ON c.upstream_id = u.id
                     WHERE u.enabled = 1
                     GROUP BY u.id, u.protocol, u.adapter_type
                     ORDER BY u.id",
                )
                .map_err(|e| AppError::Database(format!("准备 v18 凭据就绪审计失败: {e}")))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                })
                .map_err(|e| AppError::Database(format!("执行 v18 凭据就绪审计失败: {e}")))?;

            let mut candidates = Vec::new();
            for row in rows {
                let (upstream_id, protocol, adapter_type, count, kind, payload, scheme) =
                    row.map_err(|e| AppError::Database(format!("解析 v18 凭据就绪审计失败: {e}")))?;
                if count != 1 {
                    return Err(AppError::Config(format!(
                        "v18 净化被阻止：启用路由上游 {upstream_id} 必须恰有一个凭据，实际为 {count}"
                    )));
                }
                candidates.push(Candidate {
                    upstream_id,
                    protocol,
                    adapter_type,
                    credential_kind: kind.ok_or_else(|| {
                        AppError::Config("v18 凭据审计缺少 credential_kind".to_string())
                    })?,
                    encrypted_payload: payload.ok_or_else(|| {
                        AppError::Config("v18 凭据审计缺少 encrypted_payload".to_string())
                    })?,
                    encryption_scheme: scheme.ok_or_else(|| {
                        AppError::Config("v18 凭据审计缺少 encryption_scheme".to_string())
                    })?,
                });
            }
            candidates
        };

        for candidate in candidates {
            if !credential::is_ready_kind(&candidate.credential_kind) {
                return Err(AppError::Config(format!(
                    "v18 净化被阻止：启用路由上游 {} 仍使用未精确分类的凭据类型 {}",
                    candidate.upstream_id, candidate.credential_kind
                )));
            }
            if !credential::kind_can_serve(
                &candidate.credential_kind,
                &candidate.protocol,
                &candidate.adapter_type,
            ) {
                return Err(AppError::Config(format!(
                    "v18 净化被阻止：启用路由上游 {} 的凭据类型与 adapter 不兼容",
                    candidate.upstream_id
                )));
            }
            if candidate.encryption_scheme != protector.scheme() {
                return Err(AppError::Config(format!(
                    "v18 净化被阻止：启用路由上游 {} 的凭据加密方案不可用",
                    candidate.upstream_id
                )));
            }
            let plaintext = protector
                .unprotect(&candidate.encrypted_payload)
                .map_err(|_| {
                    AppError::Config(format!(
                        "v18 净化被阻止：启用路由上游 {} 的凭据无法解密",
                        candidate.upstream_id
                    ))
                })?;
            if !credential::validate_payload(&candidate.credential_kind, &plaintext) {
                return Err(AppError::Config(format!(
                    "v18 净化被阻止：启用路由上游 {} 的凭据 payload 不可直接运行",
                    candidate.upstream_id
                )));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn ensure_v18_credential_readiness_for_test(
        &self,
        protector: &dyn CredentialProtector,
    ) -> Result<(), AppError> {
        self.ensure_v18_credential_readiness_with_protector(protector)
    }

    /// 初始化数据库连接并创建表
    ///
    /// 数据库文件位于 `~/.agent-switch/agent-switch.db`
    pub fn init() -> Result<Self, AppError> {
        let db_path = get_app_config_dir().join("agent-switch.db");
        let db_exists = db_path.exists();

        // 确保父目录存在
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        let conn = Connection::open(&db_path).map_err(|e| AppError::Database(e.to_string()))?;

        // 启用外键约束
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        if !db_exists {
            // For a brand-new database, configure incremental auto-vacuum
            // before creating any tables so no rebuild is needed later.
            conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        register_db_change_hook(&conn);

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_tables()?;

        // 阶段 5 破坏性迁移编排：先迁到 v17（影子网关域完整），强制落盘纯网关
        // 回滚导出，只有写入成功后才执行 v18 purge。
        //
        // 历史逻辑会在每次升级前创建包含客户端 live 快照/旧 providers 明文 secret 的
        // 整库 `.db` 备份，并在备份失败时继续迁移；这既复制了敏感残留，也无法保证 purge
        // 前存在安全恢复点。v18 起改为严格 fail-closed 的纯网关 SQL 回滚导出。
        let stored_version = {
            let conn = lock_conn!(db.conn);
            Self::get_user_version(&conn)?
        };
        if stored_version < 17 {
            db.apply_schema_migrations_to_version(17)?;
        }
        let pre_purge_version = {
            let conn = lock_conn!(db.conn);
            Self::get_user_version(&conn)?
        };
        let performs_v18_purge = db_exists && pre_purge_version == 17 && SCHEMA_VERSION >= 18;
        if performs_v18_purge {
            {
                let conn = lock_conn!(db.conn);
                gateway_migration::reclassify_v17_credentials(&conn)?;
            }
            db.ensure_v18_credential_readiness()?;
            let rollback_path = db.backup_pure_gateway_before_v18()?;
            db.verify_local_gateway_rollback_file(&rollback_path)?;
            log::info!(
                "已在 v18 purge 前生成并验证纯网关回滚导出: {}",
                rollback_path.display()
            );
        }
        db.apply_schema_migrations()?;
        if performs_v18_purge {
            let sanitized_backups = db.reclaim_sensitive_history_after_v18()?;
            log::info!(
                "v18 purge 后历史敏感残留清理完成（主库 VACUUM，清理应用自有备份 {sanitized_backups} 个）"
            );
        }
        if let Err(e) = db.ensure_incremental_auto_vacuum() {
            log::warn!("Failed to ensure incremental auto-vacuum: {e}");
        }
        db.ensure_model_pricing_seeded()?;

        // Startup cleanup: prune old logs and reclaim space
        if let Err(e) = db.cleanup_old_stream_check_logs(7) {
            log::warn!("Startup stream_check_logs cleanup failed: {e}");
        }
        if let Err(e) = db.rollup_and_prune(30) {
            log::warn!("Startup rollup_and_prune failed: {e}");
        }
        // Reclaim disk space after cleanup
        {
            let conn = lock_conn!(db.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Startup incremental vacuum failed: {e}");
            }
        }

        Ok(db)
    }

    /// 读取磁盘上数据库的 `user_version`；仅当它比应用支持的 [`SCHEMA_VERSION`]
    /// 更新时返回 `Some(version)`。
    ///
    /// 用于初始化失败后判断是否为「数据库版本过新（应用过旧，需升级应用）」的可恢复
    /// 场景——此时不应反复弹出无效的重试对话框，而应引导用户在应用内升级。
    pub fn stored_user_version_exceeds_supported(
        db_path: &std::path::Path,
    ) -> Result<Option<i32>, AppError> {
        if !db_path.exists() {
            return Ok(None);
        }
        let conn = Connection::open(db_path).map_err(|e| AppError::Database(e.to_string()))?;
        let version = Self::get_user_version(&conn)?;
        Ok((version > SCHEMA_VERSION).then_some(version))
    }

    /// 创建内存数据库（用于测试）
    pub fn memory() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        // 启用外键约束
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(e.to_string()))?;
        register_db_change_hook(&conn);

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.create_tables()?;
        db.ensure_model_pricing_seeded()?;

        Ok(db)
    }

    pub(crate) fn get_auto_vacuum_mode(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA auto_vacuum;", [], |row| row.get(0))
            .map_err(|e| AppError::Database(format!("读取 auto_vacuum 失败: {e}")))
    }

    fn has_user_tables(conn: &Connection) -> Result<bool, AppError> {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(format!("读取表数量失败: {e}")))?;
        Ok(count > 0)
    }

    pub(crate) fn ensure_incremental_auto_vacuum_on_conn(
        conn: &Connection,
    ) -> Result<bool, AppError> {
        let mode = Self::get_auto_vacuum_mode(conn)?;
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = Self::has_user_tables(conn)?;
        conn.execute("PRAGMA auto_vacuum = INCREMENTAL;", [])
            .map_err(|e| AppError::Database(format!("设置 auto_vacuum 失败: {e}")))?;

        if !has_tables {
            return Ok(false);
        }

        conn.execute("VACUUM;", [])
            .map_err(|e| AppError::Database(format!("执行 VACUUM 失败: {e}")))?;
        conn.execute("PRAGMA foreign_keys = ON;", [])
            .map_err(|e| AppError::Database(format!("恢复 foreign_keys 失败: {e}")))?;
        Ok(true)
    }

    pub(crate) fn ensure_incremental_auto_vacuum(&self) -> Result<bool, AppError> {
        let mode = {
            let conn = lock_conn!(self.conn);
            Self::get_auto_vacuum_mode(&conn)?
        };
        if mode == 2 {
            return Ok(false);
        }

        let has_tables = {
            let conn = lock_conn!(self.conn);
            Self::has_user_tables(&conn)?
        };
        if has_tables {
            log::info!(
                "Detected auto_vacuum={mode}, rebuilding database to enable incremental vacuum"
            );
            self.backup_database_file()?;
        }

        let rebuilt = {
            let conn = lock_conn!(self.conn);
            Self::ensure_incremental_auto_vacuum_on_conn(&conn)?
        };

        if rebuilt {
            log::info!("Incremental auto-vacuum enabled after database rebuild");
        } else {
            log::info!("Incremental auto-vacuum configured for new database");
        }

        Ok(rebuilt)
    }

    /// 检查 MCP 服务器表是否为空
    pub fn is_mcp_table_empty(&self) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count == 0)
    }

    /// 检查提示词表是否为空
    pub fn is_prompts_table_empty(&self) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count == 0)
    }
}
