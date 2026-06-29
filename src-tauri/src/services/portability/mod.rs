//! 配置导入导出服务编排。
//!
//! - `export(mode, password?)`：收集 → gzip → 加密 → 装包 → JSON 文本。
//! - `import(package, password?, conflict_mode)`：解析 → 校验版本 → 解密 → gunzip → 反序列化
//!   → full_backup 前自动本地 DB 备份 → 事务内 apply。
//!
//! 双密钥策略：full_backup 用主密钥（绑定本机/同凭据环境），portable 用 Argon2id 密码派生（可跨机器）。
//! 详见 design.md §3-§5。

pub mod apply;
pub mod collect;
pub mod crypto_box;
pub mod package;

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use apply::{ApplyStrategy, ImportReport};
use collect::CollectMode;
use package::{
    ExportPackage, KdfParams, Payload, ALGO_AES_GCM, FORMAT_VERSION, KDF_ARGON2ID, KDF_NONE,
    MODE_FULL_BACKUP, MODE_PORTABLE,
};

use crate::config::paths;
use crate::services::keychain;

/// 导出结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportResult {
    /// 导出包 JSON 文本（前端触发下载）。
    pub package: String,
    /// 警告文案（如弱密码提示）。
    pub warnings: Vec<String>,
}

/// 导入结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportResult {
    pub imported: ImportReport,
    /// 导入前自动创建的本地 DB 备份路径（仅 full_backup）。
    pub pre_import_backup: Option<String>,
    pub warnings: Vec<String>,
}

/// 导出配置。
///
/// - `mode = "full_backup"`：用主密钥加密，含凭据 BLOB。主密钥不可用 → 503。
/// - `mode = "portable"`：需 `password`，Argon2id 派生密钥，脱敏。
pub fn export(
    db: &Mutex<Connection>,
    mode: &str,
    password: Option<&str>,
) -> Result<ExportResult, String> {
    let collect_mode = match mode {
        MODE_FULL_BACKUP => CollectMode::FullBackup,
        MODE_PORTABLE => CollectMode::Portable,
        other => {
            return Err(format!(
                "未知的导出模式: {}（支持 full_backup / portable）",
                other
            ))
        }
    };

    let mut warnings: Vec<String> = Vec::new();

    if collect_mode == CollectMode::Portable {
        let pwd = password
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "脱敏导出需要设置导出密码".to_string())?;
        if let Some(w) = crypto_box::weak_password_warning(pwd) {
            warnings.push(w.to_string());
        }
    }

    // 1. 收集各表 → Payload。
    let payload = collect::collect(db, collect_mode)?;

    // 2. 序列化 → gzip。
    let payload_json =
        serde_json::to_vec(&payload).map_err(|e| format!("序列化 payload 失败: {}", e))?;
    let sealed = seal_payload(&payload_json, collect_mode, password)?;

    // 3. 装包。
    let package = ExportPackage {
        format_version: FORMAT_VERSION,
        mode: mode.to_string(),
        algo: ALGO_AES_GCM.to_string(),
        kdf: sealed.kdf.to_string(),
        kdf_params: sealed.kdf_params,
        nonce: sealed.nonce_b64,
        created_at: now_iso()?,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        ciphertext: sealed.ciphertext_b64,
    };

    let package_json =
        serde_json::to_string_pretty(&package).map_err(|e| format!("序列化导出包失败: {}", e))?;

    Ok(ExportResult {
        package: package_json,
        warnings,
    })
}

/// 导入配置。
///
/// - 解析包、校验 format_version。
/// - 按 kdf 解密（主密钥 / 密码）。
/// - full_backup 前：自动复制当前 DB 文件到 `data_dir/backups/db/`。
/// - 事务内按模式 apply（replace / merge）。
pub fn import(
    db: &Mutex<Connection>,
    data_dir: &Path,
    package_json: &str,
    password: Option<&str>,
    _conflict_mode: Option<&str>,
) -> Result<ImportResult, String> {
    // 1. 解析包 + 版本校验。
    let package: ExportPackage =
        serde_json::from_str(package_json).map_err(|e| format!("导出包解析失败: {}", e))?;
    if package.format_version != FORMAT_VERSION {
        return Err(format!(
            "导出包版本不兼容（当前支持 {}，包为 {}）",
            FORMAT_VERSION, package.format_version
        ));
    }
    if package.algo != ALGO_AES_GCM {
        return Err(format!("不支持的加密算法: {}", package.algo));
    }

    // 2. 解密 + gunzip + 反序列化。
    let payload = open_payload(&package, password)?;

    // 3. full_backup 前：自动本地 DB 文件备份。
    let pre_import_backup = if package.mode == MODE_FULL_BACKUP {
        Some(backup_db_file(db, data_dir)?)
    } else {
        None
    };

    // 4. 事务内 apply。
    let strategy = match package.mode.as_str() {
        MODE_FULL_BACKUP => ApplyStrategy::Replace,
        MODE_PORTABLE => ApplyStrategy::Merge,
        other => return Err(format!("未知的导出包模式: {}", other)),
    };

    let imported = apply::apply(db, &payload, strategy)?;

    Ok(ImportResult {
        imported,
        pre_import_backup: pre_import_backup.map(|p| p.to_string_lossy().to_string()),
        warnings: Vec::new(),
    })
}

// ── 加密装包 / 解包 ─────────────────────────────────────────────────────────

struct Sealed {
    kdf: &'static str,
    kdf_params: Option<KdfParams>,
    nonce_b64: String,
    ciphertext_b64: String,
}

fn seal_payload(
    payload_json: &[u8],
    mode: CollectMode,
    password: Option<&str>,
) -> Result<Sealed, String> {
    match mode {
        CollectMode::FullBackup => {
            // 主密钥模式：load_master_key 不可用 → 503 由 handler 转。
            let master_key = keychain::load_master_key()?
                .ok_or_else(|| "系统凭据管理器不可用，无法导出完整备份".to_string())?;
            let (blob, _) = crypto_box::seal(&master_key, payload_json)?;
            let (nonce_b64, ciphertext_b64) = split_blob(&blob)?;
            Ok(Sealed {
                kdf: KDF_NONE,
                kdf_params: None,
                nonce_b64,
                ciphertext_b64,
            })
        }
        CollectMode::Portable => {
            let pwd = password.unwrap_or("");
            let salt = crypto_box::random_salt();
            let key = crypto_box::derive_key_argon2id(
                pwd,
                &salt,
                KdfParams::DEFAULT_M_COST,
                KdfParams::DEFAULT_T_COST,
                KdfParams::DEFAULT_P_COST,
            )?;
            let (blob, _) = crypto_box::seal(&key, payload_json)?;
            let (nonce_b64, ciphertext_b64) = split_blob(&blob)?;
            Ok(Sealed {
                kdf: KDF_ARGON2ID,
                kdf_params: Some(crypto_box::default_kdf_params(&salt)),
                nonce_b64,
                ciphertext_b64,
            })
        }
    }
}

fn open_payload(package: &ExportPackage, password: Option<&str>) -> Result<Payload, String> {
    let blob = reassemble_blob(&package.nonce, &package.ciphertext)?;

    let key: [u8; 32] = match package.kdf.as_str() {
        KDF_NONE => {
            // 主密钥模式：失败 → 可读错误（密钥不匹配 / 包损坏），不崩溃。
            keychain::load_master_key()?
                .ok_or_else(|| "系统主密钥不可用，无法解密完整备份".to_string())?
        }
        KDF_ARGON2ID => {
            let params = package
                .kdf_params
                .as_ref()
                .ok_or_else(|| "脱敏包缺少 KDF 参数".to_string())?;
            let pwd = password
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "脱敏包需要导出密码".to_string())?;
            crypto_box::derive_key_from_params(pwd, params)?
        }
        other => return Err(format!("不支持的 KDF 类型: {}", other)),
    };

    let payload_json = crypto_box::open(&key, &blob)?;
    let payload: Payload =
        serde_json::from_slice(&payload_json).map_err(|e| format!("导出包内容解析失败: {}", e))?;
    Ok(payload)
}

/// `seal` 返回 `nonce || ciphertext`，装包时拆分为 nonce 与 ciphertext 两段 base64。
fn split_blob(blob: &[u8]) -> Result<(String, String), String> {
    if blob.len() < 12 {
        return Err("密文过短".to_string());
    }
    let (nonce, ciphertext) = blob.split_at(12);
    Ok((
        crate::services::crypto::b64_encode(nonce),
        crate::services::crypto::b64_encode(ciphertext),
    ))
}

/// 导入时把 nonce + ciphertext 两段 base64 拼回 `nonce || ciphertext`。
fn reassemble_blob(nonce_b64: &str, ciphertext_b64: &str) -> Result<Vec<u8>, String> {
    let nonce = crate::services::crypto::b64_decode(nonce_b64)?;
    let ciphertext = crate::services::crypto::b64_decode(ciphertext_b64)?;
    if nonce.len() != 12 {
        return Err("导出包 nonce 长度无效".to_string());
    }
    let mut blob = Vec::with_capacity(nonce.len() + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

// ── 本地 DB 文件备份 ─────────────────────────────────────────────────────────

/// 复制当前 sqlite 文件到 `data_dir/backups/db/`，用于 full_backup 导入前安全网。
///
/// 参考 `tool_takeover::backup_before_write` 的文件复制范式。
/// WAL 模式下复制主库文件即可（备份是导入前的快照，导入在事务内进行）。
fn backup_db_file(db: &Mutex<Connection>, data_dir: &Path) -> Result<PathBuf, String> {
    // 确保待备份数据已落盘：在持锁状态下 checkpoint WAL。
    {
        let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| format!("WAL checkpoint 失败: {}", e))?;
    }

    let backup_root = data_dir.join("backups").join("db");
    std::fs::create_dir_all(&backup_root).map_err(|e| format!("创建 DB 备份目录失败: {}", e))?;

    let src = paths::db_path(data_dir);
    let timestamp = now_iso()?;
    // Windows 文件名不允许 ':'，替换为 '-'。
    let safe_ts = timestamp.replace(':', "-");
    let dst = backup_root.join(format!("agent-switch-{}.db", safe_ts));

    std::fs::copy(&src, &dst).map_err(|e| format!("备份数据库文件失败: {}", e))?;

    tracing::info!("完整备份导入前已备份当前数据库到 {}", dst.display());
    Ok(dst)
}

fn now_iso() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    /// 构建已迁移的内存库。
    fn fresh_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().expect("无法创建内存数据库");
        let db = Arc::new(Mutex::new(conn));
        run_migrations(&db).expect("迁移应成功");
        db
    }

    /// 插入一个带凭据 BLOB 的账号 + 端点，便于验证凭据保留/脱敏。
    fn seed_data(db: &Mutex<Connection>) {
        let conn = db.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, account_type, platform, status, credentials_encrypted, priority, created_at, updated_at)
             VALUES ('acct-1', 'Acme', 'api_key', 'anthropic', 'active', X'DEAD', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO endpoints (id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, created_at, updated_at)
             VALUES ('ep-1', 'acct-1', 'Primary', 'https://api.anthropic.com', 'anthropic', X'BEEF', 'api_key', 1, 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO endpoint_models (id, endpoint_id, model_name, display_name, source, is_available, created_at, updated_at)
             VALUES ('m-1', 'ep-1', 'claude-3-opus', 'Claude 3 Opus', 'custom', 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_takeover (tool, enabled, updated_at) VALUES ('claude-code', 1, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_metadata (key, value, updated_at) VALUES ('auto_model_refresh_enabled', 'true', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_metadata (key, value, updated_at) VALUES ('last_model_sync_at', '2024-06-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    /// full_backup round-trip：collect → seal（显式密钥）→ open → apply(replace) → 凭据恢复、接管关闭。
    #[test]
    fn full_backup_roundtrip_restores_credentials_and_disables_takeover() {
        let src = fresh_db();
        seed_data(&src);

        let master_key = [7u8; 32];
        let payload = collect::collect(&src, CollectMode::FullBackup).unwrap();

        // 验证 full_backup 含凭据 BLOB（base64）。
        assert!(payload.accounts[0].credentials_b64.is_some());
        assert!(payload.endpoints[0].api_key_b64.is_some());
        // ui_settings 只含白名单偏好键，不含 last_model_sync_at。
        let keys: Vec<&str> = payload
            .ui_settings
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert!(keys.contains(&"auto_model_refresh_enabled"));
        assert!(!keys.contains(&"last_model_sync_at"));

        // seal → open（显式密钥，绕过 keychain）。
        let payload_json = serde_json::to_vec(&payload).unwrap();
        let (blob, _) = crypto_box::seal(&master_key, &payload_json).unwrap();
        let recovered = crypto_box::open(&master_key, &blob).unwrap();
        let payload2: Payload = serde_json::from_slice(&recovered).unwrap();
        assert_eq!(payload2.accounts.len(), 1);

        // 导入到全新库（replace）。
        let dst = fresh_db();
        let report = apply::apply(&dst, &payload2, ApplyStrategy::Replace).unwrap();
        assert_eq!(report.accounts, 1);
        assert_eq!(report.endpoints, 1);

        // 凭据 BLOB 原样恢复。
        let dconn = dst.lock().unwrap();
        let cred: Option<Vec<u8>> = dconn
            .query_row(
                "SELECT credentials_encrypted FROM accounts WHERE id='acct-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cred, Some(vec![0xDE, 0xAD]));
        let key: Option<Vec<u8>> = dconn
            .query_row(
                "SELECT api_key_encrypted FROM endpoints WHERE id='ep-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(key, Some(vec![0xBE, 0xEF]));
        // 接管强制关闭。
        let enabled: i64 = dconn
            .query_row(
                "SELECT enabled FROM tool_takeover WHERE tool='claude-code'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(enabled, 0);
    }

    /// portable：collect 脱敏 → 凭据列为 None；merge 导入到已有同名端点不覆盖 api_key。
    #[test]
    fn portable_strips_credentials_and_merge_preserves_local_key() {
        let src = fresh_db();
        seed_data(&src);

        let payload = collect::collect(&src, CollectMode::Portable).unwrap();
        // 脱敏：凭据列为 None。
        assert!(payload.accounts[0].credentials_b64.is_none());
        assert!(payload.endpoints[0].api_key_b64.is_none());

        // 导入到已有同名端点的库（merge）：本机 api_key 不被覆盖。
        let dst = fresh_db();
        {
            let dconn = dst.lock().unwrap();
            // 本机已有一个同名端点，但 api_key 是不同的 BLOB。
            dconn.execute(
                "INSERT INTO endpoints (id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, created_at, updated_at)
                 VALUES ('local-ep', NULL, 'Primary', 'https://api.anthropic.com', 'anthropic', X'1234', 'api_key', 1, 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let report = apply::apply(&dst, &payload, ApplyStrategy::Merge).unwrap();
        assert_eq!(report.endpoints, 1);

        let dconn = dst.lock().unwrap();
        // 命中同名端点：本机 api_key_encrypted 保持原值 X'1234'，未被覆盖为 NULL。
        let key: Option<Vec<u8>> = dconn
            .query_row(
                "SELECT api_key_encrypted FROM endpoints WHERE name='Primary'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(key, Some(vec![0x12, 0x34]));
        // 账号为新增（本机无同名账号），凭据缺失。
        let cred: Option<Vec<u8>> = dconn
            .query_row(
                "SELECT credentials_encrypted FROM accounts WHERE name='Acme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(cred.is_none());
    }

    /// 弱密码检测。
    #[test]
    fn weak_password_detection() {
        assert!(crypto_box::weak_password_warning("123").is_some());
        assert!(crypto_box::weak_password_warning("aaaaaaa").is_some()); // 单一字符类 + 短
        assert!(crypto_box::weak_password_warning("abcdefgh").is_some()); // 单一字符类（全小写），虽 ≥8
        assert!(crypto_box::weak_password_warning("Abc1234!").is_none()); // 混合 + ≥8
    }

    /// 错误密钥解密失败给可读错误，不 panic。
    #[test]
    fn wrong_key_fails_readable() {
        let plain = b"hello world";
        let (blob, _) = crypto_box::seal(&[1u8; 32], plain).unwrap();
        let err = crypto_box::open(&[2u8; 32], &blob).unwrap_err();
        assert!(err.contains("解密失败"));
    }
}
