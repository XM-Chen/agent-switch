//! 数据库备份和恢复
//!
//! 提供 SQL 导出/导入和二进制快照备份功能。

use super::{lock_conn, Database};
use crate::config::get_app_config_dir;
use crate::error::AppError;
use crate::gateway::credential;
use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
use chrono::{Local, Utc};
use rusqlite::backup::Backup;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const AGENT_SWITCH_SQL_EXPORT_HEADER: &str = "-- Agent Switch SQLite 导出";

/// 仅含纯网关域的表，用于破坏性迁移（阶段 5）前的安全回滚导出。
///
/// 这些表的 schema 与数据构成一个自洽子集：所有外键都在集合内部
/// （upstreams <- upstream_credentials/upstream_models；gateway_models <-
/// model_aliases/route_targets <- route_target_health），导出后导入不会产生悬空引用。
/// 刻意排除一切旧客户端域表（providers/mcp_servers/skills/prompts/proxy_live_backup/
/// proxy_config/profiles/session_log_sync 等），因此恢复后不会重建客户端快照或明文 secret。
const PURE_GATEWAY_TABLES: &[&str] = &[
    "gateway_config",
    "gateway_api_keys",
    "upstreams",
    "upstream_credentials",
    "upstream_models",
    "gateway_models",
    "model_aliases",
    "route_targets",
    "route_target_health",
    "gateway_migration_report",
    "proxy_request_logs",
    "usage_daily_rollups",
    "model_pricing",
];

/// 跨机器便携同步只携带网关路由图，不携带任何本机凭据、信任状态、监听配置或运行态。
const PORTABLE_GATEWAY_TABLES: &[&str] = &[
    "upstreams",
    "upstream_models",
    "gateway_models",
    "model_aliases",
    "route_targets",
    "model_pricing",
];
const LOCAL_GATEWAY_ROLLBACK_SCOPE: &str = "local-gateway-rollback-v1";
const PORTABLE_GATEWAY_SYNC_SCOPE: &str = "portable-gateway-v1";
const LOCAL_GATEWAY_ROLLBACK_INDEXES: &[&str] = &[
    "idx_gateway_api_keys_active",
    "idx_upstreams_enabled",
    "idx_upstream_models_model",
    "idx_gateway_models_lookup",
    "idx_route_targets_order",
    "idx_gateway_migration_report_severity",
    "idx_request_logs_provider",
    "idx_request_logs_created_at",
    "idx_request_logs_model",
    "idx_request_logs_session",
    "idx_request_logs_status",
    "idx_request_logs_gateway_route",
    "idx_request_logs_app_created_at",
    "idx_request_logs_dedup_lookup_expr",
];

/// A database backup entry for the UI
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub created_at: String, // ISO 8601
}

#[derive(Debug, Clone)]
struct PortableCredentialRow {
    id: String,
    upstream_id: String,
    credential_kind: String,
    encrypted_payload: Vec<u8>,
    encryption_scheme: String,
    key_hint: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl Database {
    /// 导出为 SQLite 兼容的 SQL 文本（内存字符串，完整导出）
    pub fn export_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        Self::dump_sql(&snapshot, &[])
    }

    /// 仅导出纯网关域表的 schema 与数据，作为阶段 5 破坏性迁移前的安全回滚点。
    ///
    /// 此方法与跨机 `export_portable_gateway_sql_string` 语义严格分离：本机回滚可包含
    /// DPAPI credential 与本机信任/运行态；portable 包不包含。两者 scope/header 不同，
    /// 禁止混用。
    pub fn export_pure_gateway_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        let row_counts = Self::gateway_table_row_counts(&snapshot, PURE_GATEWAY_TABLES)?;
        let data_sha256 = Self::gateway_data_sha256(&snapshot, PURE_GATEWAY_TABLES)?;
        Self::dump_sql_whitelist(
            &snapshot,
            PURE_GATEWAY_TABLES,
            LOCAL_GATEWAY_ROLLBACK_SCOPE,
            "included-local-only",
            Some((&row_counts, &data_sha256)),
        )
    }

    /// 导出跨机器便携网关路由图（不携带凭据/本机信任状态/监听配置/运行态）。
    ///
    /// 与 `export_pure_gateway_sql_string` 的同机回滚语义严格分离：portable 包默认排除
    /// `upstream_credentials`、`gateway_api_keys`、`gateway_config`、health/logs/rollups/report，
    /// 并清空 legacy provenance 字段。此函数是 WebDAV/S3 v3 协议唯一允许上传的 SQL 来源。
    pub fn export_portable_gateway_sql_string(&self) -> Result<String, AppError> {
        let snapshot = self.snapshot_to_memory()?;
        for sql in [
            "UPDATE upstreams SET legacy_app_type = NULL, legacy_provider_id = NULL",
            "UPDATE upstream_models SET legacy_app_type = NULL, legacy_provider_id = NULL",
            "UPDATE gateway_models SET legacy_app_type = NULL, legacy_source_id = NULL",
            "UPDATE route_targets SET legacy_app_type = NULL, legacy_aggregate_id = NULL",
        ] {
            snapshot
                .execute(sql, [])
                .map_err(|e| AppError::Database(format!("清除便携导出 provenance 失败: {e}")))?;
        }
        Self::dump_sql_whitelist(
            &snapshot,
            PORTABLE_GATEWAY_TABLES,
            PORTABLE_GATEWAY_SYNC_SCOPE,
            "omitted",
            None,
        )
    }

    /// 导入跨机器便携网关路由图，仅替换六张 portable 表，保留本机凭据与运行态。
    ///
    /// 远端 SQL 先在隔离内存库中通过 SQLite authorizer 执行：只允许 portable 白名单表的
    /// CREATE/INDEX/INSERT、事务以及 foreign_keys/user_version PRAGMA；ATTACH/DETACH/ALTER/
    /// DROP/DELETE/UPDATE/函数等一律拒绝。执行后再次严格校验表集合和外键。
    ///
    /// 本机凭据仅在 upstream 的 `(base_url, protocol, adapter_type)` 身份完全不变时保留；
    /// 同 ID 身份变化或远端删除 upstream 时不继承凭据，要求用户重新录入。
    pub fn import_portable_gateway_sql_string(&self, sql_raw: &str) -> Result<(), AppError> {
        Self::validate_portable_gateway_sql_export(sql_raw)?;

        let portable = Connection::open_in_memory()
            .map_err(|e| AppError::Database(format!("创建便携导入隔离库失败: {e}")))?;
        Self::install_portable_sql_authorizer(&portable);
        let execute_result = portable.execute_batch(sql_raw);
        portable.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        execute_result.map_err(|e| {
            AppError::Database(format!("执行便携网关 SQL 失败或被安全策略拒绝: {e}"))
        })?;
        Self::validate_portable_gateway_database(&portable)?;

        let candidate = self.snapshot_to_memory()?;
        let compatible_credentials =
            Self::collect_compatible_portable_credentials(&candidate, &portable)?;

        candidate
            .execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")
            .map_err(|e| AppError::Database(format!("开启便携导入候选事务失败: {e}")))?;
        let merge_result = (|| {
            // 子表到父表删除；父表到子表写入。显式清空 credential/health（candidate
            // 为批量替换暂关 FK，不能依赖级联）：credential 随后仅为身份未变的 upstream
            // 恢复，health 对新路由图从 closed 重新开始。
            for table in [
                "route_target_health",
                "upstream_credentials",
                "model_aliases",
                "route_targets",
                "upstream_models",
                "gateway_models",
                "upstreams",
                "model_pricing",
            ] {
                candidate
                    .execute(&format!("DELETE FROM \"{table}\""), [])
                    .map_err(|e| AppError::Database(format!("清空本机便携表 {table} 失败: {e}")))?;
            }
            for table in [
                "upstreams",
                "upstream_models",
                "gateway_models",
                "model_aliases",
                "route_targets",
                "model_pricing",
            ] {
                Self::copy_table_rows(&portable, &candidate, table)?;
            }
            for credential in &compatible_credentials {
                candidate
                    .execute(
                        "INSERT INTO upstream_credentials
                            (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                             key_hint, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            credential.id,
                            credential.upstream_id,
                            credential.credential_kind,
                            credential.encrypted_payload,
                            credential.encryption_scheme,
                            credential.key_hint,
                            credential.created_at,
                            credential.updated_at,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Database(format!("恢复本机 upstream credential 失败: {e}"))
                    })?;
            }
            Self::validate_portable_candidate(&candidate)?;
            Ok::<(), AppError>(())
        })();

        match merge_result {
            Ok(()) => candidate
                .execute_batch("COMMIT; PRAGMA foreign_keys=ON;")
                .map_err(|e| AppError::Database(format!("提交便携导入候选事务失败: {e}")))?,
            Err(error) => {
                let _ = candidate.execute_batch("ROLLBACK; PRAGMA foreign_keys=ON;");
                return Err(error);
            }
        }

        // 写回前仅对真实磁盘主库生成本机安全备份；内存数据库（测试/临时候选）跳过，
        // 避免错误备份用户真实配置目录中的另一个数据库文件。
        let is_disk_database = {
            let main = lock_conn!(self.conn);
            main.path().is_some_and(|path| !path.is_empty())
        };
        if is_disk_database {
            self.backup_database_file()?;
        }
        {
            let mut main = lock_conn!(self.conn);
            let backup = Backup::new(&candidate, &mut main)
                .map_err(|e| AppError::Database(format!("创建便携导入写回任务失败: {e}")))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(format!("原子写回便携网关数据失败: {e}")))?;
        }
        Ok(())
    }

    /// 验证本机纯网关回滚文件可以安全执行、完整还原且其 DPAPI 凭据在当前用户下可解密。
    /// 该方法不修改主数据库，用于 v18 purge 前的强制恢复点验收。
    pub(crate) fn verify_local_gateway_rollback_file(&self, path: &Path) -> Result<(), AppError> {
        let sql = fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;
        let protector = PlatformCredentialProtector;
        Self::load_verified_local_gateway_rollback(&sql, &protector).map(|_| ())
    }

    /// 将 `local-gateway-rollback-v1` 安全导入当前数据库，只替换纯网关白名单表。
    /// 任意 SQL、跨机器不可解密的 DPAPI blob、行数/hash 不一致或外键损坏都会原子拒绝。
    pub fn import_local_gateway_rollback_sql_string(&self, sql: &str) -> Result<(), AppError> {
        let protector = PlatformCredentialProtector;
        self.import_local_gateway_rollback_sql_string_with_protector(sql, &protector)
    }

    pub fn import_local_gateway_rollback_file(&self, path: &Path) -> Result<(), AppError> {
        if !path.exists() {
            return Err(AppError::InvalidInput(format!(
                "本机回滚文件不存在: {}",
                path.display()
            )));
        }
        let sql = fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;
        self.import_local_gateway_rollback_sql_string(&sql)
    }

    fn import_local_gateway_rollback_sql_string_with_protector(
        &self,
        sql: &str,
        protector: &dyn CredentialProtector,
    ) -> Result<(), AppError> {
        let rollback = Self::load_verified_local_gateway_rollback(sql, protector)?;
        let expected_counts = Self::gateway_table_row_counts(&rollback, PURE_GATEWAY_TABLES)?;
        let expected_hash = Self::gateway_data_sha256(&rollback, PURE_GATEWAY_TABLES)?;
        let candidate = self.snapshot_to_memory()?;
        candidate
            .execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")
            .map_err(|e| AppError::Database(format!("开启本机回滚候选事务失败: {e}")))?;
        let merge_result = (|| {
            for table in [
                "route_target_health",
                "model_aliases",
                "route_targets",
                "upstream_credentials",
                "upstream_models",
                "gateway_models",
                "upstreams",
                "gateway_migration_report",
                "proxy_request_logs",
                "usage_daily_rollups",
                "model_pricing",
                "gateway_api_keys",
                "gateway_config",
            ] {
                candidate
                    .execute(&format!("DELETE FROM \"{table}\""), [])
                    .map_err(|e| {
                        AppError::Database(format!("清空本机回滚目标表 {table} 失败: {e}"))
                    })?;
            }
            for table in PURE_GATEWAY_TABLES {
                Self::copy_table_rows(&rollback, &candidate, table)?;
            }
            Self::validate_local_gateway_candidate(&candidate, protector)?;
            let actual_counts = Self::gateway_table_row_counts(&candidate, PURE_GATEWAY_TABLES)?;
            let actual_hash = Self::gateway_data_sha256(&candidate, PURE_GATEWAY_TABLES)?;
            if actual_counts != expected_counts || actual_hash != expected_hash {
                return Err(AppError::Database(
                    "本机回滚候选写入后完整性校验失败".to_string(),
                ));
            }
            Ok::<(), AppError>(())
        })();
        match merge_result {
            Ok(()) => candidate
                .execute_batch("COMMIT; PRAGMA foreign_keys=ON;")
                .map_err(|e| AppError::Database(format!("提交本机回滚候选失败: {e}")))?,
            Err(error) => {
                let _ = candidate.execute_batch("ROLLBACK; PRAGMA foreign_keys=ON;");
                return Err(error);
            }
        }

        let is_disk_database = {
            let main = lock_conn!(self.conn);
            main.path().is_some_and(|path| !path.is_empty())
        };
        if is_disk_database {
            self.backup_database_file()?;
        }
        let mut main = lock_conn!(self.conn);
        let backup = Backup::new(&candidate, &mut main)
            .map_err(|e| AppError::Database(format!("创建本机回滚写回任务失败: {e}")))?;
        backup
            .step(-1)
            .map_err(|e| AppError::Database(format!("原子写回本机网关回滚失败: {e}")))?;
        Ok(())
    }

    fn load_verified_local_gateway_rollback(
        sql: &str,
        protector: &dyn CredentialProtector,
    ) -> Result<Connection, AppError> {
        let (expected_counts, expected_hash) = Self::validate_local_gateway_sql_export(sql)?;
        let rollback = Connection::open_in_memory()
            .map_err(|e| AppError::Database(format!("创建本机回滚隔离库失败: {e}")))?;
        Self::install_local_gateway_sql_authorizer(&rollback);
        let execute_result = rollback.execute_batch(sql);
        rollback.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        execute_result.map_err(|e| {
            AppError::Database(format!("执行本机网关回滚 SQL 失败或被安全策略拒绝: {e}"))
        })?;
        Self::validate_local_gateway_database(
            &rollback,
            &expected_counts,
            &expected_hash,
            protector,
        )?;
        Ok(rollback)
    }

    /// 在 v18 破坏性 purge 前，强制将纯网关回滚导出落盘。
    ///
    /// 与历史整库 `.db` 备份不同，此文件严格不含客户端快照/旧 providers 明文 secret。
    /// 调用方必须在数据库已迁到 v17（影子域完整）后调用；任何写入失败都应阻止 v18 迁移。
    pub(crate) fn backup_pure_gateway_before_v18(&self) -> Result<PathBuf, AppError> {
        let backup_dir = get_app_config_dir().join("backups");
        fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;

        let base_id = format!(
            "gateway_rollback_v17_before_v18_{}",
            Local::now().format("%Y%m%d_%H%M%S")
        );
        let mut backup_path = backup_dir.join(format!("{base_id}.sql"));
        let mut counter = 1;
        while backup_path.exists() {
            backup_path = backup_dir.join(format!("{base_id}_{counter}.sql"));
            counter += 1;
        }

        let dump = self.export_pure_gateway_sql_string()?;
        crate::config::atomic_write(&backup_path, dump.as_bytes())?;
        Ok(backup_path)
    }

    /// v18 purge 后清除当前数据库及应用自有历史整库备份中的敏感残留。
    ///
    /// 当前数据库：先 WAL checkpoint(TRUNCATE)，再 VACUUM 重写文件，清除 WAL/SHM/freelist
    /// 中可能残留的客户端快照与明文 secret。应用自有备份：仅处理标准命名
    /// `backups/db_backup_*.db`，逐库删除 live snapshot、清空旧 providers JSON、删除接管/
    /// 明文 token setting、冻结 proxy_config 并 VACUUM。外部 WebDAV/S3 历史包不在本方法
    /// 所有权范围内，不能由程序宣称已删除。
    pub(crate) fn reclaim_sensitive_history_after_v18(&self) -> Result<usize, AppError> {
        {
            let conn = lock_conn!(self.conn);
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
                .map_err(|e| AppError::Database(format!("清理主数据库历史页失败: {e}")))?;
        }

        let backup_dir = get_app_config_dir().join("backups");
        if !backup_dir.exists() {
            return Ok(0);
        }

        let mut sanitized = 0usize;
        for entry in fs::read_dir(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))? {
            let entry = entry.map_err(|e| AppError::io(&backup_dir, e))?;
            let path = entry.path();
            let is_owned_db_backup = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("db_backup_"))
                && path.extension().is_some_and(|ext| ext == "db");
            if !is_owned_db_backup {
                continue;
            }
            Self::sanitize_owned_database_backup(&path)?;
            sanitized += 1;
        }
        Ok(sanitized)
    }

    fn sanitize_owned_database_backup(path: &Path) -> Result<(), AppError> {
        let conn = Connection::open(path).map_err(|e| {
            AppError::Database(format!("打开历史数据库备份 {} 失败: {e}", path.display()))
        })?;
        conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE;")
            .map_err(|e| {
                AppError::Database(format!("开启历史备份清理事务 {} 失败: {e}", path.display()))
            })?;

        let result = (|| {
            if Self::table_exists(&conn, "proxy_live_backup")? {
                conn.execute("DELETE FROM proxy_live_backup", [])
                    .map_err(|e| AppError::Database(format!("清除备份 live snapshot 失败: {e}")))?;
            }
            if Self::table_exists(&conn, "providers")? {
                // 历史整库备份不再参与运行时迁移；为避免遗漏未知 secret 键，保守清空
                // settings_config/meta 正文，同时保留 provider 行和外键结构以供审计。
                conn.execute(
                    "UPDATE providers SET settings_config = '{}', meta = '{}'",
                    [],
                )
                .map_err(|e| AppError::Database(format!("清空备份 providers JSON 失败: {e}")))?;
            }
            if Self::table_exists(&conn, "settings")? {
                conn.execute(
                    "DELETE FROM settings
                     WHERE key LIKE 'proxy_takeover_%'
                        OR key LIKE 'auto_failover_enabled_%'
                        OR key = 'claude_desktop_gateway_token'",
                    [],
                )
                .map_err(|e| AppError::Database(format!("清除备份接管/token setting 失败: {e}")))?;
            }
            if Self::table_exists(&conn, "proxy_config")? {
                if Self::has_column(&conn, "proxy_config", "live_takeover_active")? {
                    conn.execute("UPDATE proxy_config SET live_takeover_active = 0", [])
                        .map_err(|e| {
                            AppError::Database(format!("冻结备份 live takeover 失败: {e}"))
                        })?;
                }
                if Self::has_column(&conn, "proxy_config", "enabled")? {
                    conn.execute("UPDATE proxy_config SET enabled = 0", [])
                        .map_err(|e| {
                            AppError::Database(format!("冻结备份 proxy enabled 失败: {e}"))
                        })?;
                }
                if Self::has_column(&conn, "proxy_config", "route_mode")? {
                    conn.execute("UPDATE proxy_config SET route_mode = 'direct'", [])
                        .map_err(|e| {
                            AppError::Database(format!("重置备份 route_mode 失败: {e}"))
                        })?;
                }
            }
            Ok::<(), AppError>(())
        })();

        match result {
            Ok(()) => {
                conn.execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
                    .map_err(|e| {
                        AppError::Database(format!(
                            "提交并重写历史数据库备份 {} 失败: {e}",
                            path.display()
                        ))
                    })?;
                Ok(())
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(AppError::Database(format!(
                    "清理历史数据库备份 {} 失败: {error}",
                    path.display()
                )))
            }
        }
    }

    /// 导出为 SQLite 兼容的 SQL 文本
    pub fn export_sql(&self, target_path: &Path) -> Result<(), AppError> {
        let dump = self.export_sql_string()?;

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }

        crate::config::atomic_write(target_path, dump.as_bytes())
    }

    /// 从 SQL 文件导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql(&self, source_path: &Path) -> Result<String, AppError> {
        if !source_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "SQL 文件不存在: {}",
                source_path.display()
            )));
        }

        let sql_raw = fs::read_to_string(source_path).map_err(|e| AppError::io(source_path, e))?;
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        self.import_sql_string(sql_content)
    }

    /// 从 SQL 字符串导入，返回生成的备份 ID（若无备份则为空字符串）
    pub fn import_sql_string(&self, sql_raw: &str) -> Result<String, AppError> {
        self.import_sql_string_inner(sql_raw)
    }

    fn import_sql_string_inner(&self, sql_raw: &str) -> Result<String, AppError> {
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        Self::validate_agent_switch_sql_export(sql_content)?;

        // 导入前备份现有数据库
        let backup_path = self.backup_database_file()?;

        // 在临时数据库执行导入，确保失败不会污染主库
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "创建临时数据库文件失败".to_string(),
            source: e,
        })?;
        let temp_path = temp_file.path().to_path_buf();
        let temp_conn =
            Connection::open(&temp_path).map_err(|e| AppError::Database(e.to_string()))?;

        temp_conn
            .execute_batch(sql_content)
            .map_err(|e| AppError::Database(format!("执行 SQL 导入失败: {e}")))?;

        // 补齐缺失表/索引并进行基础校验
        Self::create_tables_on_conn(&temp_conn)?;
        Self::apply_schema_migrations_on_conn(&temp_conn)?;
        Self::validate_basic_state(&temp_conn)?;

        // 使用 Backup 将临时库原子写回主库
        {
            let mut main_conn = lock_conn!(self.conn);
            let backup = Backup::new(&temp_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let backup_id = backup_path
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        Ok(backup_id)
    }

    /// 创建内存快照以避免长时间持有数据库锁
    pub(crate) fn snapshot_to_memory(&self) -> Result<Connection, AppError> {
        let conn = lock_conn!(self.conn);
        let mut snapshot =
            Connection::open_in_memory().map_err(|e| AppError::Database(e.to_string()))?;

        {
            let backup =
                Backup::new(&conn, &mut snapshot).map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(snapshot)
    }

    fn validate_agent_switch_sql_export(sql: &str) -> Result<(), AppError> {
        let trimmed = sql.trim_start();
        if trimmed.starts_with(AGENT_SWITCH_SQL_EXPORT_HEADER) {
            return Ok(());
        }

        Err(AppError::localized(
            "backup.sql.invalid_format",
            "仅支持导入由 Agent Switch 导出的 SQL 备份文件。",
            "Only SQL backups exported by Agent Switch are supported.",
        ))
    }

    fn validate_portable_gateway_sql_export(sql: &str) -> Result<(), AppError> {
        Self::validate_agent_switch_sql_export(sql)?;
        let header = sql.lines().take(8).collect::<Vec<_>>().join("\n");
        if !header.contains("-- scope: portable-gateway-v1")
            || !header.contains("-- credentials: omitted")
        {
            return Err(AppError::InvalidInput(
                "拒绝导入非 portable-gateway-v1 或携带凭据的 SQL".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_local_gateway_sql_export(
        sql: &str,
    ) -> Result<(BTreeMap<String, i64>, String), AppError> {
        Self::validate_agent_switch_sql_export(sql)?;
        let header = sql.lines().take(10).collect::<Vec<_>>();
        if !header
            .iter()
            .any(|line| line.trim() == "-- scope: local-gateway-rollback-v1")
            || !header
                .iter()
                .any(|line| line.trim() == "-- credentials: included-local-only")
        {
            return Err(AppError::InvalidInput(
                "拒绝导入非 local-gateway-rollback-v1 或未标明本机凭据的 SQL".to_string(),
            ));
        }
        let row_counts_raw = header
            .iter()
            .find_map(|line| line.strip_prefix("-- row-counts: "))
            .ok_or_else(|| AppError::InvalidInput("本机回滚缺少 row-counts 完整性清单".into()))?;
        let row_counts: BTreeMap<String, i64> = serde_json::from_str(row_counts_raw)
            .map_err(|e| AppError::InvalidInput(format!("本机回滚 row-counts 无效: {e}")))?;
        let expected_tables = PURE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<std::collections::BTreeSet<_>>();
        if row_counts
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != expected_tables
        {
            return Err(AppError::InvalidInput(
                "本机回滚 row-counts 表集合不符合纯网关白名单".into(),
            ));
        }
        if row_counts.values().any(|count| *count < 0) {
            return Err(AppError::InvalidInput(
                "本机回滚 row-counts 包含负数".into(),
            ));
        }
        let data_hash = header
            .iter()
            .find_map(|line| line.strip_prefix("-- data-sha256: "))
            .filter(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
            .ok_or_else(|| AppError::InvalidInput("本机回滚缺少有效 data-sha256".into()))?;
        Ok((row_counts, data_hash.to_ascii_lowercase()))
    }

    fn install_local_gateway_sql_authorizer(conn: &Connection) {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

        let allowed_tables = PURE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<std::collections::HashSet<_>>();
        let allowed_indexes = LOCAL_GATEWAY_ROLLBACK_INDEXES
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let allow_coalesce = true;
        conn.authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::CreateTable { table_name } => {
                if allowed_tables.contains(table_name) || table_name == "sqlite_sequence" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::CreateIndex {
                index_name,
                table_name,
            } => {
                let allowed = allowed_tables.contains(table_name)
                    && (index_name.starts_with("sqlite_autoindex_")
                        || allowed_indexes.contains(index_name));
                if allowed {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Insert { table_name } | AuthAction::Read { table_name, .. } => {
                if allowed_tables.contains(table_name) || table_name == "sqlite_master" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Update { table_name, .. } => {
                if table_name == "sqlite_master" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Pragma {
                pragma_name,
                pragma_value: _,
            } => {
                if matches!(pragma_name, "foreign_keys" | "user_version") {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Reindex { index_name } => {
                if allowed_indexes.contains(index_name) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Function { function_name }
                if allow_coalesce && function_name.eq_ignore_ascii_case("coalesce") =>
            {
                Authorization::Allow
            }
            AuthAction::Transaction { .. } | AuthAction::Select | AuthAction::Recursive => {
                Authorization::Allow
            }
            _ => Authorization::Deny,
        }));
    }

    fn install_portable_sql_authorizer(conn: &Connection) {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

        let allowed_tables: std::collections::HashSet<String> = PORTABLE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect();
        let allowed_indexes: std::collections::HashSet<&'static str> = [
            "idx_upstreams_enabled",
            "idx_upstream_models_model",
            "idx_gateway_models_lookup",
            "idx_route_targets_order",
        ]
        .into_iter()
        .collect();
        conn.authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::CreateTable { table_name } => {
                if allowed_tables.contains(table_name) || table_name == "sqlite_sequence" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::CreateIndex { table_name, .. } => {
                if allowed_tables.contains(table_name) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Insert { table_name } | AuthAction::Read { table_name, .. } => {
                if allowed_tables.contains(table_name) || table_name == "sqlite_master" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Update { table_name, .. } => {
                if table_name == "sqlite_master" {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Pragma {
                pragma_name,
                pragma_value: _,
            } => {
                if matches!(pragma_name, "foreign_keys" | "user_version") {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Reindex { index_name } => {
                if allowed_indexes.contains(index_name) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Transaction { .. } | AuthAction::Select | AuthAction::Recursive => {
                Authorization::Allow
            }
            _ => Authorization::Deny,
        }));
    }

    fn validate_local_gateway_database(
        conn: &Connection,
        expected_counts: &BTreeMap<String, i64>,
        expected_hash: &str,
        protector: &dyn CredentialProtector,
    ) -> Result<(), AppError> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|e| AppError::Database(format!("读取本机回滚表集合失败: {e}")))?;
        let actual = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Database(format!("查询本机回滚表集合失败: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("收集本机回滚表集合失败: {e}")))?;
        let mut expected = PURE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<Vec<_>>();
        expected.sort();
        if actual != expected {
            return Err(AppError::InvalidInput(format!(
                "本机回滚表集合不符合白名单，expected={expected:?}, actual={actual:?}"
            )));
        }
        let actual_counts = Self::gateway_table_row_counts(conn, PURE_GATEWAY_TABLES)?;
        if &actual_counts != expected_counts {
            return Err(AppError::InvalidInput(format!(
                "本机回滚行数校验失败，expected={expected_counts:?}, actual={actual_counts:?}"
            )));
        }
        let actual_hash = Self::gateway_data_sha256(conn, PURE_GATEWAY_TABLES)?;
        if actual_hash != expected_hash {
            return Err(AppError::InvalidInput(
                "本机回滚 data-sha256 校验失败".to_string(),
            ));
        }
        Self::validate_local_gateway_candidate(conn, protector)
    }

    fn validate_local_gateway_candidate(
        conn: &Connection,
        protector: &dyn CredentialProtector,
    ) -> Result<(), AppError> {
        let fk_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .map_err(|e| AppError::Database(format!("检查本机回滚外键失败: {e}")))?;
        if fk_violations != 0 {
            return Err(AppError::InvalidInput(format!(
                "本机回滚包含 {fk_violations} 个外键违规"
            )));
        }

        let mut stmt = conn
            .prepare(
                "SELECT c.upstream_id, c.credential_kind, c.encrypted_payload,
                        c.encryption_scheme, u.protocol, u.adapter_type,
                        EXISTS(
                            SELECT 1 FROM route_targets r
                            JOIN gateway_models g ON g.id = r.gateway_model_id
                            WHERE r.upstream_id = u.id
                              AND r.enabled = 1
                              AND u.enabled = 1
                              AND g.enabled = 1
                              AND g.migration_status = 'active'
                        ) AS required_by_active_route
                 FROM upstream_credentials c
                 JOIN upstreams u ON u.id = c.upstream_id
                 ORDER BY c.upstream_id, c.credential_kind",
            )
            .map_err(|e| AppError::Database(format!("准备验证本机回滚凭据失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, bool>(6)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("查询本机回滚凭据失败: {e}")))?;
        for row in rows {
            let (
                upstream_id,
                kind,
                encrypted,
                scheme,
                protocol,
                adapter_type,
                required_by_active_route,
            ) = row.map_err(|e| AppError::Database(format!("解析本机回滚凭据失败: {e}")))?;
            // 历史 v17 可能保留 disabled/orphan upstream 的不可运行凭据。这些行作为
            // provenance 可以安全回滚；只有活跃路由真正依赖的凭据才必须满足运行时矩阵。
            if required_by_active_route
                && (!credential::is_ready_kind(&kind)
                    || !credential::kind_can_serve(&kind, &protocol, &adapter_type))
            {
                return Err(AppError::InvalidInput(format!(
                    "本机回滚上游 {upstream_id} 的凭据类型不可直接运行"
                )));
            }
            if scheme != protector.scheme() {
                return Err(AppError::InvalidInput(format!(
                    "本机回滚上游 {upstream_id} 的凭据不是当前用户可用的保护方案"
                )));
            }
            let plaintext = protector.unprotect(&encrypted).map_err(|_| {
                AppError::InvalidInput(format!(
                    "本机回滚上游 {upstream_id} 的 DPAPI 凭据无法在当前用户下解密"
                ))
            })?;
            if !credential::validate_payload(&kind, &plaintext) {
                return Err(AppError::InvalidInput(format!(
                    "本机回滚上游 {upstream_id} 的凭据 payload 无效"
                )));
            }
        }
        Self::validate_portable_candidate(conn)
    }

    fn validate_portable_gateway_database(conn: &Connection) -> Result<(), AppError> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|e| AppError::Database(format!("读取便携导入表集合失败: {e}")))?;
        let actual = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Database(format!("查询便携导入表集合失败: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Database(format!("收集便携导入表集合失败: {e}")))?;
        let mut expected = PORTABLE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<Vec<_>>();
        expected.sort();
        if actual != expected {
            return Err(AppError::InvalidInput(format!(
                "便携导入表集合不符合白名单，expected={expected:?}, actual={actual:?}"
            )));
        }
        let fk_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .map_err(|e| AppError::Database(format!("检查便携导入外键失败: {e}")))?;
        if fk_violations != 0 {
            return Err(AppError::InvalidInput(format!(
                "便携导入包含 {fk_violations} 个外键违规"
            )));
        }
        Ok(())
    }

    fn collect_compatible_portable_credentials(
        local: &Connection,
        portable: &Connection,
    ) -> Result<Vec<PortableCredentialRow>, AppError> {
        let mut portable_upstreams = std::collections::HashMap::new();
        {
            let mut stmt = portable
                .prepare("SELECT id, base_url, protocol, adapter_type FROM upstreams")
                .map_err(|e| AppError::Database(format!("读取 portable upstream 身份失败: {e}")))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(|e| AppError::Database(format!("查询 portable upstream 身份失败: {e}")))?;
            for row in rows {
                let (id, base_url, protocol, adapter_type) = row.map_err(|e| {
                    AppError::Database(format!("解析 portable upstream 身份失败: {e}"))
                })?;
                portable_upstreams.insert(id, (base_url, protocol, adapter_type));
            }
        }

        let mut stmt = local
            .prepare(
                "SELECT c.id, c.upstream_id, c.credential_kind, c.encrypted_payload,
                        c.encryption_scheme, c.key_hint, c.created_at, c.updated_at,
                        u.base_url, u.protocol, u.adapter_type
                 FROM upstream_credentials c
                 JOIN upstreams u ON u.id = c.upstream_id",
            )
            .map_err(|e| AppError::Database(format!("读取本机 upstream credential 失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    PortableCredentialRow {
                        id: row.get(0)?,
                        upstream_id: row.get(1)?,
                        credential_kind: row.get(2)?,
                        encrypted_payload: row.get(3)?,
                        encryption_scheme: row.get(4)?,
                        key_hint: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    },
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("查询本机 upstream credential 失败: {e}")))?;

        let mut compatible = Vec::new();
        for row in rows {
            let (credential, base_url, protocol, adapter_type) = row.map_err(|e| {
                AppError::Database(format!("解析本机 upstream credential 失败: {e}"))
            })?;
            let Some(remote_identity) = portable_upstreams.get(&credential.upstream_id) else {
                continue;
            };
            if remote_identity == &(base_url, protocol, adapter_type) {
                compatible.push(credential);
            }
        }
        Ok(compatible)
    }

    fn copy_table_rows(
        source_conn: &Connection,
        target_conn: &Connection,
        table: &str,
    ) -> Result<(), AppError> {
        let columns = Self::get_table_columns(source_conn, table)?;
        let placeholders = (1..=columns.len())
            .map(|idx| format!("?{idx}"))
            .collect::<Vec<_>>()
            .join(", ");
        let cols = columns
            .iter()
            .map(|column| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!("INSERT INTO \"{table}\" ({cols}) VALUES ({placeholders})");
        let mut stmt = source_conn
            .prepare(&format!("SELECT * FROM \"{table}\""))
            .map_err(|e| AppError::Database(format!("读取 portable 表 {table} 失败: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(format!("查询 portable 表 {table} 失败: {e}")))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let mut values = Vec::with_capacity(columns.len());
            for idx in 0..columns.len() {
                values.push(
                    row.get::<_, rusqlite::types::Value>(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?,
                );
            }
            target_conn
                .execute(&insert_sql, rusqlite::params_from_iter(values.iter()))
                .map_err(|e| AppError::Database(format!("写入 portable 表 {table} 失败: {e}")))?;
        }
        Ok(())
    }

    fn validate_portable_candidate(conn: &Connection) -> Result<(), AppError> {
        let fk_violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .map_err(|e| AppError::Database(format!("检查便携候选外键失败: {e}")))?;
        if fk_violations != 0 {
            return Err(AppError::InvalidInput(format!(
                "便携候选包含 {fk_violations} 个外键违规"
            )));
        }
        let unroutable_enabled_models: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_models gm
                 WHERE gm.enabled = 1
                   AND NOT EXISTS(
                       SELECT 1 FROM route_targets rt
                       JOIN upstreams u ON u.id = rt.upstream_id
                       WHERE rt.gateway_model_id = gm.id AND rt.enabled = 1 AND u.enabled = 1
                   )",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(format!("检查便携候选路由状态失败: {e}")))?;
        if unroutable_enabled_models != 0 {
            return Err(AppError::InvalidInput(format!(
                "便携候选包含 {unroutable_enabled_models} 个已启用但无可用路由的模型"
            )));
        }
        Ok(())
    }

    /// Periodic maintenance: prune old stream-check logs and roll up usage, then
    /// incremental-vacuum. Auto DB-file backup is retired in the gateway (old
    /// whole-DB backups may still carry client Live snapshots); this method keeps
    /// only the maintenance half for compatibility callers.
    #[cfg(test)]
    pub(crate) fn periodic_backup_if_needed(&self) -> Result<(), AppError> {
        let interval_hours = self
            .get_setting("backup_interval_hours")?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(24);
        if interval_hours > 0 {
            let backup_dir = get_app_config_dir().join("backups");
            if !backup_dir.exists() {
                self.backup_database_file()?;
            } else {
                let latest = fs::read_dir(&backup_dir).ok().and_then(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
                        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                        .max()
                });

                let interval_secs = u64::from(interval_hours) * 3600;
                let needs_backup = match latest {
                    None => true,
                    Some(last_modified) => {
                        last_modified.elapsed().unwrap_or_default()
                            > std::time::Duration::from_secs(interval_secs)
                    }
                };

                if needs_backup {
                    log::info!(
                        "Periodic backup: latest backup is older than {interval_hours} hours, creating new backup"
                    );
                    self.backup_database_file()?;
                }
            }
        }

        // Periodic gateway usage maintenance is always enabled.
        let mut reclaimed_rows = 0u64;
        match self.rollup_and_prune(30) {
            Ok(deleted) => {
                reclaimed_rows += deleted;
            }
            Err(e) => {
                log::warn!("Periodic rollup_and_prune failed: {e}");
            }
        }
        if reclaimed_rows > 0 {
            let conn = lock_conn!(self.conn);
            if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
                log::warn!("Periodic incremental vacuum failed: {e}");
            }
        }

        Ok(())
    }

    /// 生成一致性快照备份，返回备份文件路径（不存在主库时返回 None）
    pub(crate) fn backup_database_file(&self) -> Result<Option<PathBuf>, AppError> {
        let db_path = get_app_config_dir().join("agent-switch.db");
        if !db_path.exists() {
            return Ok(None);
        }

        let backup_dir = db_path
            .parent()
            .ok_or_else(|| AppError::Config("无效的数据库路径".to_string()))?
            .join("backups");

        fs::create_dir_all(&backup_dir).map_err(|e| AppError::io(&backup_dir, e))?;

        let base_id = format!("db_backup_{}", Local::now().format("%Y%m%d_%H%M%S"));
        let mut backup_id = base_id.clone();
        let mut backup_path = backup_dir.join(format!("{backup_id}.db"));
        let mut counter = 1;
        while backup_path.exists() {
            backup_id = format!("{base_id}_{counter}");
            backup_path = backup_dir.join(format!("{backup_id}.db"));
            counter += 1;
        }

        {
            let conn = lock_conn!(self.conn);
            let mut dest_conn =
                Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;
            let backup = Backup::new(&conn, &mut dest_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Self::cleanup_db_backups(&backup_dir)?;
        Ok(Some(backup_path))
    }

    /// 清理旧的数据库备份，保留最新的 N 个
    fn cleanup_db_backups(dir: &Path) -> Result<(), AppError> {
        let retain = crate::settings::effective_backup_retain_count();
        let entries = match fs::read_dir(dir) {
            Ok(iter) => iter
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .map(|ext| ext == "db")
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>(),
            Err(_) => return Ok(()),
        };

        if entries.len() <= retain {
            return Ok(());
        }

        let remove_count = entries.len().saturating_sub(retain);
        let mut sorted = entries;
        sorted.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());

        for entry in sorted.into_iter().take(remove_count) {
            if let Err(err) = fs::remove_file(entry.path()) {
                log::warn!("删除旧数据库备份失败 {}: {}", entry.path().display(), err);
            }
        }
        Ok(())
    }

    /// 基础状态校验
    fn validate_basic_state(conn: &Connection) -> Result<(), AppError> {
        let provider_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mcp_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |row| row.get(0))
            .map_err(|e| AppError::Database(e.to_string()))?;

        if provider_count == 0 && mcp_count == 0 {
            return Err(AppError::Config(
                "导入的 SQL 未包含有效的供应商或 MCP 数据".to_string(),
            ));
        }
        Ok(())
    }

    /// 导出数据库为 SQL 文本
    fn dump_sql(conn: &Connection, skip_tables: &[&str]) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap_or(0);

        output.push_str(&format!(
            "-- Agent Switch SQLite 导出\n-- 生成时间: {timestamp}\n-- user_version: {user_version}\n"
        ));
        output.push_str("PRAGMA foreign_keys=OFF;\n");
        output.push_str(&format!("PRAGMA user_version={user_version};\n"));
        output.push_str("BEGIN TRANSACTION;\n");

        // 导出 schema
        let mut stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, sql
                 FROM sqlite_master
                 WHERE sql NOT NULL AND type IN ('table','index','trigger','view')
                 ORDER BY type='table' DESC, name",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut tables = Vec::new();
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(e.to_string()))?;
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let obj_type: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
            let name: String = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
            let sql: String = row.get(3).map_err(|e| AppError::Database(e.to_string()))?;

            // 跳过 SQLite 内部对象（如 sqlite_sequence）
            if name.starts_with("sqlite_") {
                continue;
            }

            output.push_str(&sql);
            output.push_str(";\n");

            if obj_type == "table" && !name.starts_with("sqlite_") {
                tables.push(name);
            }
        }

        // 导出数据
        for table in tables {
            if skip_tables.iter().any(|t| *t == table) {
                continue;
            }
            let columns = Self::get_table_columns(conn, &table)?;
            if columns.is_empty() {
                continue;
            }

            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(e.to_string()))?;

            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    values.push(Self::format_sql_value(value)?);
                }

                let cols = columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!(
                    "INSERT INTO \"{table}\" ({cols}) VALUES ({});\n",
                    values.join(", ")
                ));
            }
        }

        output.push_str("COMMIT;\nPRAGMA foreign_keys=ON;\n");
        Ok(output)
    }

    /// 仅导出白名单表的 schema 与数据。
    ///
    /// 与 `dump_sql`（导出全部 schema、按黑名单跳过数据）不同：本方法只发射
    /// `include_tables` 中表及其附属索引/触发器的 CREATE 语句，且只为这些表发射
    /// INSERT。`sqlite_master` 中索引的 `tbl_name` 指向其归属表，故按 `tbl_name`
    /// 过滤即可同时覆盖表与索引。
    fn dump_sql_whitelist(
        conn: &Connection,
        include_tables: &[&str],
        scope: &str,
        credentials: &str,
        integrity: Option<(&BTreeMap<String, i64>, &str)>,
    ) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap_or(0);

        output.push_str(&format!(
            "-- Agent Switch SQLite 导出\n-- 生成时间: {timestamp}\n-- user_version: {user_version}\n-- scope: {scope}\n-- credentials: {credentials}\n"
        ));
        if let Some((row_counts, data_sha256)) = integrity {
            let row_counts_json = serde_json::to_string(row_counts)
                .map_err(|e| AppError::Database(format!("序列化回滚行数清单失败: {e}")))?;
            output.push_str(&format!(
                "-- row-counts: {row_counts_json}\n-- data-sha256: {data_sha256}\n"
            ));
        }
        output.push_str("PRAGMA foreign_keys=OFF;\n");
        output.push_str(&format!("PRAGMA user_version={user_version};\n"));
        output.push_str("BEGIN TRANSACTION;\n");

        let include_set: std::collections::HashSet<&str> = include_tables.iter().copied().collect();

        // 仅导出白名单表的 schema（含其索引）
        let mut stmt = conn
            .prepare(
                "SELECT type, name, tbl_name, sql
                 FROM sqlite_master
                 WHERE sql NOT NULL AND type IN ('table','index','trigger','view')
                 ORDER BY type='table' DESC, name",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut exported_tables: Vec<String> = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            let obj_type: String = row.get(0).map_err(|e| AppError::Database(e.to_string()))?;
            let name: String = row.get(1).map_err(|e| AppError::Database(e.to_string()))?;
            let tbl_name: String = row.get(2).map_err(|e| AppError::Database(e.to_string()))?;
            let sql: String = row.get(3).map_err(|e| AppError::Database(e.to_string()))?;

            if name.starts_with("sqlite_") {
                continue;
            }
            // 表对象：tbl_name == name；索引/触发器：tbl_name 指向归属表。
            if !include_set.contains(tbl_name.as_str()) {
                continue;
            }

            output.push_str(&sql);
            output.push_str(";\n");

            if obj_type == "table" {
                exported_tables.push(name);
            }
        }

        // 仅为白名单表导出数据
        for table in &exported_tables {
            let columns = Self::get_table_columns(conn, table)?;
            if columns.is_empty() {
                continue;
            }
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .map_err(|e| AppError::Database(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                let mut values = Vec::with_capacity(columns.len());
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    values.push(Self::format_sql_value(value)?);
                }
                let cols = columns
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!(
                    "INSERT INTO \"{table}\" ({cols}) VALUES ({});\n",
                    values.join(", ")
                ));
            }
        }

        output.push_str("COMMIT;\nPRAGMA foreign_keys=ON;\n");
        Ok(output)
    }

    fn gateway_table_row_counts(
        conn: &Connection,
        tables: &[&str],
    ) -> Result<BTreeMap<String, i64>, AppError> {
        let mut counts = BTreeMap::new();
        for table in tables {
            let count = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|e| AppError::Database(format!("统计回滚表 {table} 行数失败: {e}")))?;
            counts.insert((*table).to_string(), count);
        }
        Ok(counts)
    }

    fn gateway_data_sha256(conn: &Connection, tables: &[&str]) -> Result<String, AppError> {
        let mut hasher = Sha256::new();
        for table in tables {
            hasher.update(table.as_bytes());
            hasher.update([0]);
            let columns = Self::get_table_columns(conn, table)?;
            let order = columns
                .iter()
                .map(|column| format!("quote(\"{column}\")"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY {order}"))
                .map_err(|e| AppError::Database(format!("准备哈希回滚表 {table} 失败: {e}")))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| AppError::Database(format!("查询哈希回滚表 {table} 失败: {e}")))?;
            while let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
                for idx in 0..columns.len() {
                    let value = row
                        .get_ref(idx)
                        .map_err(|e| AppError::Database(e.to_string()))?;
                    match value {
                        ValueRef::Null => hasher.update([0]),
                        ValueRef::Integer(value) => {
                            hasher.update([1]);
                            hasher.update(value.to_le_bytes());
                        }
                        ValueRef::Real(value) => {
                            hasher.update([2]);
                            hasher.update(value.to_bits().to_le_bytes());
                        }
                        ValueRef::Text(value) => {
                            hasher.update([3]);
                            hasher.update((value.len() as u64).to_le_bytes());
                            hasher.update(value);
                        }
                        ValueRef::Blob(value) => {
                            hasher.update([4]);
                            hasher.update((value.len() as u64).to_le_bytes());
                            hasher.update(value);
                        }
                    }
                }
                hasher.update([0xff]);
            }
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn get_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AppError> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(|e| AppError::Database(e.to_string()))?;
        let iter = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut columns = Vec::new();
        for col in iter {
            columns.push(col.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(columns)
    }

    /// 格式化 SQL 值
    fn format_sql_value(value: ValueRef<'_>) -> Result<String, AppError> {
        match value {
            ValueRef::Null => Ok("NULL".to_string()),
            ValueRef::Integer(i) => Ok(i.to_string()),
            ValueRef::Real(f) => Ok(f.to_string()),
            ValueRef::Text(t) => {
                let text = std::str::from_utf8(t)
                    .map_err(|e| AppError::Database(format!("文本字段不是有效的 UTF-8: {e}")))?;
                let escaped = text.replace('\'', "''");
                Ok(format!("'{escaped}'"))
            }
            ValueRef::Blob(bytes) => {
                let mut s = String::from("X'");
                for b in bytes {
                    use std::fmt::Write;
                    let _ = write!(&mut s, "{b:02X}");
                }
                s.push('\'');
                Ok(s)
            }
        }
    }

    /// List all database backup files, sorted by creation time (newest first)
    pub fn list_backups() -> Result<Vec<BackupEntry>, AppError> {
        let backup_dir = get_app_config_dir().join("backups");
        if !backup_dir.exists() {
            return Ok(vec![]);
        }

        let mut entries: Vec<BackupEntry> = fs::read_dir(&backup_dir)
            .map_err(|e| AppError::io(&backup_dir, e))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "db").unwrap_or(false))
            .filter_map(|e| {
                let metadata = e.metadata().ok()?;
                let filename = e.file_name().to_string_lossy().to_string();
                let size_bytes = metadata.len();
                let created_at = metadata
                    .modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();
                Some(BackupEntry {
                    filename,
                    size_bytes,
                    created_at,
                })
            })
            .collect();

        // Sort by created_at descending (newest first)
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entries)
    }

    /// Restore database from a backup file. Returns the safety backup ID.
    pub fn restore_from_backup(&self, filename: &str) -> Result<String, AppError> {
        // Security: validate filename to prevent path traversal
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_dir = get_app_config_dir().join("backups");
        let backup_path = backup_dir.join(filename);

        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        // Step 1: Create safety backup of current database
        let safety_backup = self.backup_database_file()?;
        let safety_id = safety_backup
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        // Step 2: Open the backup file and restore it to the main database
        let source_conn =
            Connection::open(&backup_path).map_err(|e| AppError::Database(e.to_string()))?;

        {
            let mut main_conn = lock_conn!(self.conn);
            let backup = Backup::new(&source_conn, &mut main_conn)
                .map_err(|e| AppError::Database(e.to_string()))?;
            backup
                .step(-1)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        // Step 3: Run schema migrations (backup may be from an older version)
        self.create_tables()?;
        self.apply_schema_migrations()?;
        self.ensure_model_pricing_seeded()?;

        log::info!("Database restored from backup: {filename}, safety backup: {safety_id}");
        Ok(safety_id)
    }

    /// Rename a backup file. Returns the new filename.
    pub fn rename_backup(old_filename: &str, new_name: &str) -> Result<String, AppError> {
        // Validate old filename (path traversal + .db suffix)
        if old_filename.contains("..")
            || old_filename.contains('/')
            || old_filename.contains('\\')
            || !old_filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        // Clean new name
        let trimmed = new_name.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput(
                "New name cannot be empty".to_string(),
            ));
        }

        // Length limit (without .db suffix)
        let name_part = trimmed.strip_suffix(".db").unwrap_or(trimmed);
        if name_part.len() > 100 {
            return Err(AppError::InvalidInput(
                "Name too long (max 100 characters)".to_string(),
            ));
        }

        // Prevent path traversal in new name
        if name_part.contains("..")
            || name_part.contains('/')
            || name_part.contains('\\')
            || name_part.contains('\0')
        {
            return Err(AppError::InvalidInput(
                "Invalid characters in new name".to_string(),
            ));
        }

        let new_filename = format!("{name_part}.db");

        let backup_dir = get_app_config_dir().join("backups");
        let old_path = backup_dir.join(old_filename);
        let new_path = backup_dir.join(&new_filename);

        if !old_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {old_filename}"
            )));
        }

        if new_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "A backup named '{new_filename}' already exists"
            )));
        }

        fs::rename(&old_path, &new_path).map_err(|e| AppError::io(&old_path, e))?;
        log::info!("Renamed backup: {old_filename} -> {new_filename}");
        Ok(new_filename)
    }

    /// Delete a backup file permanently.
    pub fn delete_backup(filename: &str) -> Result<(), AppError> {
        // Validate filename (path traversal + .db suffix)
        if filename.contains("..")
            || filename.contains('/')
            || filename.contains('\\')
            || !filename.ends_with(".db")
        {
            return Err(AppError::InvalidInput(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_path = get_app_config_dir().join("backups").join(filename);
        if !backup_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Backup file not found: {filename}"
            )));
        }

        fs::remove_file(&backup_path).map_err(|e| AppError::io(&backup_path, e))?;
        log::info!("Deleted backup: {filename}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, PORTABLE_GATEWAY_TABLES};
    use crate::error::AppError;
    use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
    use crate::settings::{update_settings, AppSettings};
    use rusqlite::Connection;
    use serial_test::serial;

    struct TestProtector;

    impl CredentialProtector for TestProtector {
        fn scheme(&self) -> &'static str {
            "test-local-rollback-v1"
        }

        fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(plaintext.iter().map(|byte| byte ^ 0xa5).collect())
        }

        fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            self.protect(ciphertext)
        }
    }

    #[test]
    #[serial]
    fn periodic_maintenance_runs_even_when_auto_backup_disabled() -> Result<(), AppError> {
        let old_test_home = std::env::var_os("AGENT_SWITCH_TEST_HOME");
        let test_home =
            std::env::temp_dir().join("agent-switch-periodic-maintenance-backup-disabled-test");
        let _ = std::fs::remove_dir_all(&test_home);
        std::fs::create_dir_all(&test_home).expect("create test home");
        std::env::set_var("AGENT_SWITCH_TEST_HOME", &test_home);

        let settings = AppSettings {
            backup_interval_hours: Some(0),
            ..AppSettings::default()
        };
        update_settings(settings).expect("disable auto backup");

        let db = Database::memory()?;
        let now = chrono::Utc::now().timestamp();
        let old_ts = now - 40 * 86400;

        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model,
                    input_tokens, output_tokens, total_cost_usd,
                    latency_ms, status_code, created_at
                ) VALUES ('old-req', 'p1', 'claude', 'claude-3', 100, 50, '0.01', 100, 200, ?1)",
                [old_ts],
            )?;
        }

        db.periodic_backup_if_needed()?;

        let (remaining_request_logs, rollups): (i64, i64) = {
            let conn = crate::database::lock_conn!(db.conn);
            let remaining_request_logs =
                conn.query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |row| {
                    row.get(0)
                })?;
            let rollups =
                conn.query_row("SELECT COUNT(*) FROM usage_daily_rollups", [], |row| {
                    row.get(0)
                })?;
            (remaining_request_logs, rollups)
        };

        assert_eq!(
            remaining_request_logs, 0,
            "old request logs should still be pruned when auto backup is disabled"
        );
        assert_eq!(rollups, 1, "old request logs should be rolled up");

        match old_test_home {
            Some(value) => std::env::set_var("AGENT_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("AGENT_SWITCH_TEST_HOME"),
        }

        Ok(())
    }

    /// 纯网关回滚导出必须包含网关域数据，且严格排除客户端 live 快照与明文 secret。
    /// 这是阶段 5 破坏性 purge 前安全回滚点的核心安全不变量。
    #[test]
    fn pure_gateway_export_excludes_client_snapshots_and_secrets() -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            // 纯网关域数据：应进入导出
            conn.execute(
                "INSERT INTO upstreams (id, name, protocol, adapter_type, created_at, updated_at)
                 VALUES ('up-rollback', 'Rollback Upstream', 'anthropic', 'anthropic', 1, 1)",
                [],
            )?;
            // 旧客户端域：客户端 live 配置快照（含敏感原文）
            conn.execute(
                "INSERT INTO proxy_live_backup (app_type, original_config, backed_up_at)
                 VALUES ('claude', 'SECRET_CLIENT_CONFIG_SNAPSHOT', '2026-01-01')",
                [],
            )?;
            // 旧 providers 表：settings_config 历史含明文 API key
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('legacy-p', 'claude', 'Legacy', '{\"api_key\":\"sk-secret-plaintext\"}', '{}')",
                [],
            )?;
        }

        let sql = db.export_pure_gateway_sql_string()?;
        assert!(
            sql.contains("-- scope: local-gateway-rollback-v1"),
            "本机回滚导出必须使用独立 scope，禁止与 portable 包混用"
        );
        assert!(
            sql.contains("-- credentials: included-local-only"),
            "本机回滚导出应明确标注包含本机凭据材料"
        );

        // 纯网关数据进入导出
        assert!(
            sql.contains("'up-rollback'"),
            "纯网关导出应包含 upstreams 数据"
        );
        // SQLite 在 sqlite_master 中以规范化形式存储 schema（剥离 IF NOT EXISTS），
        // 故按 "CREATE TABLE upstreams (" 断言上游表 schema 进入导出。
        assert!(
            sql.contains("CREATE TABLE upstreams ("),
            "纯网关导出应包含 upstreams schema"
        );

        // 客户端快照与明文 secret 严格排除
        assert!(
            !sql.contains("SECRET_CLIENT_CONFIG_SNAPSHOT"),
            "纯网关导出绝不可包含客户端 live 配置快照"
        );
        assert!(
            !sql.contains("sk-secret-plaintext"),
            "纯网关导出绝不可包含旧 providers 明文 secret"
        );
        assert!(
            !sql.contains("INSERT INTO \"proxy_live_backup\""),
            "纯网关导出绝不可包含 proxy_live_backup 数据"
        );
        assert!(
            !sql.contains("INSERT INTO \"providers\""),
            "纯网关导出绝不可包含 providers 数据"
        );

        Ok(())
    }

    /// 本机回滚导入必须验证 scope、表集合、行数/hash 和凭据可解密性，并原子恢复纯网关表。
    #[test]
    fn local_gateway_rollback_round_trip_is_verified_and_atomic() -> Result<(), AppError> {
        let source = Database::memory()?;
        let encrypted = TestProtector.protect(b"sk-local-secret")?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute(
                "INSERT INTO upstreams
                    (id, name, enabled, base_url, protocol, adapter_type, created_at, updated_at)
                 VALUES ('rollback-up', 'Rollback', 1, 'https://rollback.invalid',
                         'openai_responses', 'codex', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     key_hint, created_at, updated_at)
                 VALUES ('rollback-cred', 'rollback-up', 'bearer_token', ?1, ?2,
                         'sk-l...cret', 1, 1)",
                rusqlite::params![encrypted, TestProtector.scheme()],
            )?;
            conn.execute(
                "INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status,
                     created_at, updated_at)
                 VALUES ('rollback-gm', 'rollback-model', 'Rollback Model', 1,
                         'manual', 'active', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     created_at, updated_at)
                 VALUES ('rollback-route', 'rollback-gm', 'rollback-up',
                         'upstream-model', 0, 1, 1, 1)",
                [],
            )?;
        }
        let sql = source.export_pure_gateway_sql_string()?;
        let target = Database::memory()?;
        target.import_local_gateway_rollback_sql_string_with_protector(&sql, &TestProtector)?;
        let conn = crate::database::lock_conn!(target.conn);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM route_targets WHERE id = 'rollback-route'",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            1
        );
        drop(conn);

        let tampered = sql.replacen("'upstream-model'", "'tampered-model'", 1);
        assert!(target
            .import_local_gateway_rollback_sql_string_with_protector(&tampered, &TestProtector)
            .is_err());
        let conn = crate::database::lock_conn!(target.conn);
        let target_model: String = conn.query_row(
            "SELECT target_model FROM route_targets WHERE id = 'rollback-route'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(target_model, "upstream-model", "失败导入不得污染主库");
        Ok(())
    }

    #[test]
    fn local_gateway_rollback_allows_incompatible_disabled_upstream_route_provenance(
    ) -> Result<(), AppError> {
        let source = Database::memory()?;
        let orphan_encrypted = TestProtector.protect(b"orphan-secret")?;
        let healthy_encrypted = TestProtector.protect(b"healthy-secret")?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute_batch(
                "INSERT INTO upstreams
                    (id, name, enabled, protocol, adapter_type, created_at, updated_at)
                 VALUES
                    ('healthy-up', 'Healthy', 1, 'openai_responses', 'codex', 1, 1),
                    ('orphan-up', 'Orphan', 0, 'unknown', 'unsupported', 1, 1);
                 INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status,
                     created_at, updated_at)
                 VALUES ('gm', 'gateway-model', 'Gateway Model', 1, 'manual', 'active', 1, 1);
                 INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     created_at, updated_at)
                 VALUES
                    ('healthy-route', 'gm', 'healthy-up', 'healthy-model', 0, 1, 1, 1),
                    ('orphan-route', 'gm', 'orphan-up', 'orphan-model', 1, 1, 1, 1);",
            )?;
            conn.execute(
                "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     created_at, updated_at)
                 VALUES
                    ('healthy-cred', 'healthy-up', 'bearer_token', ?1, ?3, 1, 1),
                    ('orphan-cred', 'orphan-up', 'x_api_key', ?2, ?3, 1, 1)",
                rusqlite::params![healthy_encrypted, orphan_encrypted, TestProtector.scheme()],
            )?;
        }

        let sql = source.export_pure_gateway_sql_string()?;
        Database::load_verified_local_gateway_rollback(&sql, &TestProtector)?;
        Ok(())
    }

    #[test]
    fn local_gateway_rollback_rejects_wrong_scope_and_wrong_dpapi_user() -> Result<(), AppError> {
        let source = Database::memory()?;
        let encrypted = TestProtector.protect(b"secret")?;
        {
            let conn = crate::database::lock_conn!(source.conn);
            conn.execute(
                "INSERT INTO upstreams
                    (id, name, enabled, base_url, protocol, adapter_type, created_at, updated_at)
                 VALUES ('up', 'Up', 0, 'https://up.invalid', 'anthropic', 'claude', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     created_at, updated_at)
                 VALUES ('cred', 'up', 'x_api_key', ?1, ?2, 1, 1)",
                rusqlite::params![encrypted, TestProtector.scheme()],
            )?;
        }
        let local = source.export_pure_gateway_sql_string()?;
        assert!(Database::load_verified_local_gateway_rollback(
            &local,
            &PlatformCredentialProtector
        )
        .is_err());
        let portable = source.export_portable_gateway_sql_string()?;
        assert!(Database::load_verified_local_gateway_rollback(&portable, &TestProtector).is_err());
        Ok(())
    }

    /// 便携网关导出只包含路由图，严格排除本机凭据、API key、设备配置、运行态和旧客户端域。
    #[test]
    fn portable_gateway_export_contains_only_routing_graph_without_credentials(
    ) -> Result<(), AppError> {
        let db = Database::memory()?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO upstreams
                    (id, name, enabled, base_url, protocol, adapter_type, legacy_app_type,
                     legacy_provider_id, created_at, updated_at)
                 VALUES ('portable-up', 'PORTABLE_UPSTREAM', 1, 'https://portable.invalid',
                         'anthropic', 'anthropic', 'LEGACY_APP_SENTINEL',
                         'LEGACY_PROVIDER_SENTINEL', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     key_hint, created_at, updated_at)
                 VALUES ('portable-cred', 'portable-up', 'x_api_key', X'44504150495F534543524554',
                         'dpapi-current-user-v1', 'DPAPI_HINT_SENTINEL', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO upstream_models
                    (upstream_id, model_id, source, refreshed_at, legacy_app_type, legacy_provider_id)
                 VALUES ('portable-up', 'portable-vendor-model', 'manual', 1,
                         'LEGACY_APP_SENTINEL', 'LEGACY_PROVIDER_SENTINEL')",
                [],
            )?;
            conn.execute(
                "INSERT INTO gateway_models
                    (id, model_id, display_name, enabled, source, migration_status,
                     legacy_app_type, legacy_source_id, created_at, updated_at)
                 VALUES ('portable-gm', 'portable-model', 'Portable Model', 1, 'manual', 'active',
                         'LEGACY_APP_SENTINEL', 'LEGACY_SOURCE_SENTINEL', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO model_aliases (alias, gateway_model_id, created_at)
                 VALUES ('portable-alias', 'portable-gm', 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     legacy_app_type, legacy_aggregate_id, created_at, updated_at)
                 VALUES ('portable-route', 'portable-gm', 'portable-up', 'portable-vendor-model',
                         0, 1, 'LEGACY_APP_SENTINEL', 'LEGACY_AGG_SENTINEL', 1, 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO gateway_api_keys
                    (id, name, key_hash, key_prefix, created_at)
                 VALUES ('local-key', 'Local Key', 'KEY_HASH_SENTINEL', 'agsk_local', 1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, meta)
                 VALUES ('legacy-portable-provider', 'claude', 'Legacy',
                         '{\"token\":\"LEGACY_PROVIDER_SECRET\"}', '{}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO mcp_servers (id, name, server_config)
                 VALUES ('mcp-portable', 'MCP', '{\"token\":\"MCP_SECRET\"}')",
                [],
            )?;
            conn.execute(
                "INSERT INTO prompts (id, app_type, name, content)
                 VALUES ('prompt-portable', 'claude', 'Prompt', 'PROMPT_SENTINEL')",
                [],
            )?;
            conn.execute(
                "INSERT INTO profiles (id, name, payload)
                 VALUES ('profile-portable', 'Profile', '{\"secret\":\"PROFILE_SENTINEL\"}')",
                [],
            )?;
        }

        let sql = db.export_portable_gateway_sql_string()?;
        assert!(sql.contains("-- scope: portable-gateway-v1"));
        assert!(sql.contains("-- credentials: omitted"));
        assert!(sql.contains("PORTABLE_UPSTREAM"));
        assert!(sql.contains("portable-route"));

        for forbidden in [
            "DPAPI_SECRET",
            "DPAPI_HINT_SENTINEL",
            "KEY_HASH_SENTINEL",
            "LEGACY_PROVIDER_SECRET",
            "MCP_SECRET",
            "PROMPT_SENTINEL",
            "PROFILE_SENTINEL",
            "LEGACY_APP_SENTINEL",
            "LEGACY_PROVIDER_SENTINEL",
            "LEGACY_SOURCE_SENTINEL",
            "LEGACY_AGG_SENTINEL",
        ] {
            assert!(
                !sql.contains(forbidden),
                "portable SQL 泄漏禁止 sentinel: {forbidden}"
            );
        }
        for forbidden_table in [
            "providers",
            "mcp_servers",
            "prompts",
            "skills",
            "profiles",
            "provider_models",
            "custom_aggregates",
            "upstream_credentials",
            "gateway_api_keys",
            "gateway_config",
            "route_target_health",
            "gateway_migration_report",
            "proxy_request_logs",
            "usage_daily_rollups",
        ] {
            assert!(
                !sql.contains(&format!("CREATE TABLE {forbidden_table} ("))
                    && !sql.contains(&format!("INSERT INTO \"{forbidden_table}\"")),
                "portable SQL 不得包含表 {forbidden_table}"
            );
        }

        // portable SQL 可独立导入，其表集合严格等于白名单且外键完整。
        let portable = Connection::open_in_memory()?;
        portable.execute_batch(&sql)?;
        let mut stmt = portable.prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let tables = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut expected = PORTABLE_GATEWAY_TABLES
            .iter()
            .map(|table| (*table).to_string())
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(tables, expected, "portable SQL 表集合必须严格等于白名单");
        let fk_violations: i64 =
            portable.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        assert_eq!(fk_violations, 0, "portable SQL 不得存在外键违规");

        Ok(())
    }

    fn seed_portable_route_graph(
        db: &Database,
        base_url: &str,
        model_id: &str,
    ) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO upstreams
                (id, name, enabled, base_url, protocol, adapter_type, created_at, updated_at)
             VALUES ('shared-up', 'Shared Upstream', 1, ?1, 'anthropic', 'anthropic', 1, 1)",
            [base_url],
        )?;
        conn.execute(
            "INSERT INTO upstream_models (upstream_id, model_id, source, refreshed_at)
             VALUES ('shared-up', ?1, 'manual', 1)",
            [model_id],
        )?;
        conn.execute(
            "INSERT INTO gateway_models
                (id, model_id, display_name, enabled, source, migration_status, created_at, updated_at)
             VALUES ('shared-gm', ?1, 'Shared Model', 1, 'manual', 'active', 1, 1)",
            [model_id],
        )?;
        conn.execute(
            "INSERT INTO route_targets
                (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
             VALUES ('shared-route', 'shared-gm', 'shared-up', ?1, 0, 1, 1, 1)",
            [model_id],
        )?;
        Ok(())
    }

    fn seed_local_gateway_state(db: &Database, base_url: &str) -> Result<(), AppError> {
        seed_portable_route_graph(db, base_url, "old-local-model")?;
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO upstream_credentials
                (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                 key_hint, created_at, updated_at)
             VALUES ('local-cred', 'shared-up', 'x_api_key', X'01020304',
                     'dpapi-current-user-v1', 'local-hint', 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO gateway_api_keys
                (id, name, key_hash, key_prefix, created_at)
             VALUES ('local-api-key', 'Local API Key', 'local-key-hash', 'agsk_local', 1)",
            [],
        )?;
        conn.execute(
            "UPDATE gateway_config SET listen_port = 49999, auth_required = 1 WHERE id = 1",
            [],
        )?;
        conn.execute(
            "INSERT INTO route_target_health
                (route_target_id, state, consecutive_failures, consecutive_successes, updated_at)
             VALUES ('shared-route', 'open', 4, 0, 1)",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn portable_import_replaces_routes_but_preserves_compatible_local_credentials_and_trust(
    ) -> Result<(), AppError> {
        let local = Database::memory()?;
        seed_local_gateway_state(&local, "https://same.invalid")?;
        let remote = Database::memory()?;
        seed_portable_route_graph(&remote, "https://same.invalid", "new-portable-model")?;

        let sql = remote.export_portable_gateway_sql_string()?;
        local.import_portable_gateway_sql_string(&sql)?;

        let conn = crate::database::lock_conn!(local.conn);
        let model: String = conn.query_row(
            "SELECT model_id FROM gateway_models WHERE id = 'shared-gm'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(model, "new-portable-model", "路由图应被 portable 数据替换");
        let credential: (Vec<u8>, String) = conn.query_row(
            "SELECT encrypted_payload, key_hint FROM upstream_credentials
             WHERE id = 'local-cred'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(credential, (vec![1, 2, 3, 4], "local-hint".to_string()));
        let local_key_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM gateway_api_keys WHERE id = 'local-api-key'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(local_key_count, 1, "本机 gateway API key 应保留");
        let listen_port: i64 = conn.query_row(
            "SELECT listen_port FROM gateway_config WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(listen_port, 49999, "本机 listener 配置应保留");
        let health_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM route_target_health", [], |row| {
                row.get(0)
            })?;
        assert_eq!(health_count, 0, "导入新路由图后 health 应重置");
        Ok(())
    }

    #[test]
    fn portable_import_drops_credential_when_upstream_identity_changes() -> Result<(), AppError> {
        let local = Database::memory()?;
        seed_local_gateway_state(&local, "https://old.invalid")?;
        let remote = Database::memory()?;
        seed_portable_route_graph(&remote, "https://new.invalid", "new-portable-model")?;

        local.import_portable_gateway_sql_string(&remote.export_portable_gateway_sql_string()?)?;
        let conn = crate::database::lock_conn!(local.conn);
        let credential_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM upstream_credentials WHERE upstream_id = 'shared-up'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            credential_count, 0,
            "同 ID upstream 身份变化时绝不可继承旧凭据，应要求重新录入"
        );
        Ok(())
    }

    #[test]
    fn portable_import_rejects_wrong_scope_and_malicious_sql_atomically() -> Result<(), AppError> {
        let local = Database::memory()?;
        seed_local_gateway_state(&local, "https://local.invalid")?;
        let remote = Database::memory()?;
        seed_portable_route_graph(&remote, "https://remote.invalid", "remote-model")?;

        let local_rollback = remote.export_pure_gateway_sql_string()?;
        assert!(
            local
                .import_portable_gateway_sql_string(&local_rollback)
                .is_err(),
            "本机回滚 scope 不得被 portable import 接受"
        );

        let valid = remote.export_portable_gateway_sql_string()?;
        let malicious = valid.replacen(
            "BEGIN TRANSACTION;",
            "BEGIN TRANSACTION; ATTACH DATABASE ':memory:' AS evil;",
            1,
        );
        assert!(
            local
                .import_portable_gateway_sql_string(&malicious)
                .is_err(),
            "authorizer 必须拒绝 ATTACH"
        );
        let extra_table = valid.replacen(
            "BEGIN TRANSACTION;",
            "BEGIN TRANSACTION; CREATE TABLE evil(secret TEXT);",
            1,
        );
        assert!(
            local
                .import_portable_gateway_sql_string(&extra_table)
                .is_err(),
            "authorizer 必须拒绝白名单外表"
        );

        let conn = crate::database::lock_conn!(local.conn);
        let unchanged_model: String = conn.query_row(
            "SELECT model_id FROM gateway_models WHERE id = 'shared-gm'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(
            unchanged_model, "old-local-model",
            "任何导入失败都不得部分改写本机路由图"
        );
        let credential_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM upstream_credentials WHERE id = 'local-cred'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(credential_count, 1, "失败时本机凭据必须保持不变");
        Ok(())
    }
}
