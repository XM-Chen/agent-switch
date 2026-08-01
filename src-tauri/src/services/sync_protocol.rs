//! Transport-agnostic portable gateway sync protocol.
//!
//! Shared by WebDAV, S3, and future transports. Artifact set is exactly `db.sql`.
//! Credentials, local trust state, client domains, Skills, and runtime state are omitted.

use std::collections::BTreeMap;
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

// ─── Protocol constants ──────────────────────────────────────

/// Portable gateway wire-format identifier. Old v2 `cc-switch-webdav-sync` manifests are
/// deliberately incompatible and may only be displayed as legacy metadata; they are never applied.
pub(crate) const PROTOCOL_FORMAT: &str = "agent-switch-gateway-sync";
pub(crate) const PROTOCOL_VERSION: u32 = 3;
pub(crate) const GATEWAY_DATA_VERSION: u32 = 1;
pub(crate) const REMOTE_DB_SQL: &str = "db.sql";
pub(crate) const REMOTE_MANIFEST: &str = "manifest.json";
pub(crate) const MAX_DEVICE_NAME_LEN: usize = 64;
pub(crate) const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_SYNC_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

// ─── Error helpers ───────────────────────────────────────────

pub(crate) fn localized(
    key: &'static str,
    zh: impl Into<String>,
    en: impl Into<String>,
) -> AppError {
    AppError::localized(key, zh, en)
}

pub(crate) fn io_context_localized(
    _key: &'static str,
    zh: impl Into<String>,
    en: impl Into<String>,
    source: std::io::Error,
) -> AppError {
    let zh_msg = zh.into();
    let en_msg = en.into();
    AppError::IoContext {
        context: format!("{zh_msg} ({en_msg})"),
        source,
    }
}

// ─── Types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncManifest {
    pub format: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_compat_version: Option<u32>,
    pub device_name: String,
    pub created_at: String,
    pub artifacts: BTreeMap<String, ArtifactMeta>,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArtifactMeta {
    pub sha256: String,
    pub size: u64,
}

pub(crate) struct LocalSnapshot {
    pub db_sql: Vec<u8>,
    pub manifest_bytes: Vec<u8>,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteLayout {
    Current,
    Legacy,
}

impl RemoteLayout {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Legacy => "legacy",
        }
    }
}

// ─── Snapshot building ───────────────────────────────────────

pub(crate) fn build_local_snapshot(
    db: &crate::database::Database,
) -> Result<LocalSnapshot, AppError> {
    let db_sql = db.export_portable_gateway_sql_string()?.into_bytes();

    let mut artifacts = BTreeMap::new();
    artifacts.insert(
        REMOTE_DB_SQL.to_string(),
        ArtifactMeta {
            sha256: sha256_hex(&db_sql),
            size: db_sql.len() as u64,
        },
    );

    let snapshot_id = compute_snapshot_id(&artifacts);
    let manifest = SyncManifest {
        format: PROTOCOL_FORMAT.to_string(),
        version: PROTOCOL_VERSION,
        db_compat_version: Some(GATEWAY_DATA_VERSION),
        device_name: detect_system_device_name().unwrap_or_else(|| "Unknown Device".to_string()),
        created_at: Utc::now().to_rfc3339(),
        artifacts,
        snapshot_id,
    };
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|e| AppError::JsonSerialize { source: e })?;
    let manifest_hash = sha256_hex(&manifest_bytes);

    Ok(LocalSnapshot {
        db_sql,
        manifest_bytes,
        manifest_hash,
    })
}

// ─── Manifest handling ───────────────────────────────────────

/// Compute a deterministic snapshot identity from artifact hashes.
///
/// BTreeMap iteration order is sorted by key, ensuring stability.
pub(crate) fn compute_snapshot_id(artifacts: &BTreeMap<String, ArtifactMeta>) -> String {
    let parts: Vec<String> = artifacts
        .iter()
        .map(|(name, meta)| format!("{}:{}", name, meta.sha256))
        .collect();
    sha256_hex(parts.join("|").as_bytes())
}

pub(crate) fn effective_db_compat_version(
    manifest: &SyncManifest,
    _layout: RemoteLayout,
) -> Option<u32> {
    manifest.db_compat_version
}

pub(crate) fn validate_manifest_compat(
    manifest: &SyncManifest,
    layout: RemoteLayout,
) -> Result<(), AppError> {
    if layout != RemoteLayout::Current {
        return Err(localized(
            "sync.legacy_manifest_blocked",
            "检测到旧版 v2 同步包；其中可能包含客户端域、Skills 或本机凭据，当前版本禁止自动导入。请在可信旧版本中转换为 portable gateway v3 包。",
            "A legacy v2 sync package was found. It may contain client domains, Skills, or local credentials and cannot be imported automatically. Convert it to a portable gateway v3 package using a trusted older version.",
        ));
    }
    if manifest.format != PROTOCOL_FORMAT {
        return Err(localized(
            "sync.manifest_format_incompatible",
            format!("远端 manifest 格式不兼容: {}", manifest.format),
            format!(
                "Remote manifest format is incompatible: {}",
                manifest.format
            ),
        ));
    }
    if manifest.version != PROTOCOL_VERSION {
        return Err(localized(
            "sync.manifest_version_incompatible",
            format!(
                "远端 manifest 协议版本不兼容: v{} (本地 v{PROTOCOL_VERSION})",
                manifest.version
            ),
            format!(
                "Remote manifest protocol version is incompatible: v{} (local v{PROTOCOL_VERSION})",
                manifest.version
            ),
        ));
    }
    if manifest.db_compat_version != Some(GATEWAY_DATA_VERSION) {
        return Err(localized(
            "sync.manifest_gateway_data_version_incompatible",
            format!(
                "远端网关数据版本不兼容: {:?} (本地 gateway-v{GATEWAY_DATA_VERSION})",
                manifest.db_compat_version
            ),
            format!(
                "Remote gateway data version is incompatible: {:?} (local gateway-v{GATEWAY_DATA_VERSION})",
                manifest.db_compat_version
            ),
        ));
    }

    let expected_artifacts = [REMOTE_DB_SQL.to_string()]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let actual_artifacts = manifest
        .artifacts
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if actual_artifacts != expected_artifacts {
        return Err(localized(
            "sync.manifest_artifact_set_incompatible",
            format!(
                "远端 artifact 集合不符合 portable gateway 契约: {actual_artifacts:?}"
            ),
            format!(
                "Remote artifact set does not match the portable gateway contract: {actual_artifacts:?}"
            ),
        ));
    }
    let expected_snapshot_id = compute_snapshot_id(&manifest.artifacts);
    if manifest.snapshot_id != expected_snapshot_id {
        return Err(localized(
            "sync.manifest_snapshot_id_mismatch",
            "远端 snapshotId 与 artifact 哈希不匹配",
            "Remote snapshotId does not match the artifact hashes.",
        ));
    }
    Ok(())
}

// ─── Artifact verification ───────────────────────────────────

pub(crate) fn validate_artifact_size_limit(artifact_name: &str, size: u64) -> Result<(), AppError> {
    if size > MAX_SYNC_ARTIFACT_BYTES {
        let max_mb = MAX_SYNC_ARTIFACT_BYTES / 1024 / 1024;
        return Err(localized(
            "sync.artifact_too_large",
            format!("artifact {artifact_name} 超过下载上限（{} MB）", max_mb),
            format!(
                "Artifact {artifact_name} exceeds download limit ({} MB)",
                max_mb
            ),
        ));
    }
    Ok(())
}

/// Verify that downloaded artifact bytes match the expected size and SHA-256 hash.
pub(crate) fn verify_artifact(
    bytes: &[u8],
    artifact_name: &str,
    meta: &ArtifactMeta,
) -> Result<(), AppError> {
    // Quick size check before expensive hash
    if bytes.len() as u64 != meta.size {
        return Err(localized(
            "sync.artifact_size_mismatch",
            format!(
                "artifact {artifact_name} 大小不匹配 (expected: {}, got: {})",
                meta.size,
                bytes.len(),
            ),
            format!(
                "Artifact {artifact_name} size mismatch (expected: {}, got: {})",
                meta.size,
                bytes.len(),
            ),
        ));
    }

    let actual_hash = sha256_hex(bytes);
    if actual_hash != meta.sha256 {
        return Err(localized(
            "sync.artifact_hash_mismatch",
            format!(
                "artifact {artifact_name} SHA256 校验失败 (expected: {}..., got: {}...)",
                meta.sha256.get(..8).unwrap_or(&meta.sha256),
                actual_hash.get(..8).unwrap_or(&actual_hash),
            ),
            format!(
                "Artifact {artifact_name} SHA256 verification failed (expected: {}..., got: {}...)",
                meta.sha256.get(..8).unwrap_or(&meta.sha256),
                actual_hash.get(..8).unwrap_or(&actual_hash),
            ),
        ));
    }
    Ok(())
}

// ─── Snapshot application ────────────────────────────────────

pub(crate) fn apply_snapshot(
    db: &crate::database::Database,
    db_sql: &[u8],
) -> Result<(), AppError> {
    let sql_str = std::str::from_utf8(db_sql).map_err(|e| {
        localized(
            "sync.sql_not_utf8",
            format!("SQL 非 UTF-8: {e}"),
            format!("SQL is not valid UTF-8: {e}"),
        )
    })?;
    db.import_portable_gateway_sql_string(sql_str)
}

// ─── Utilities ───────────────────────────────────────────────

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn detect_system_device_name() -> Option<String> {
    let env_name = ["AGENT_SWITCH_DEVICE_NAME", "COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|value| normalize_device_name(&value));

    if env_name.is_some() {
        return env_name;
    }

    let output = Command::new("hostname").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let hostname = String::from_utf8(output.stdout).ok()?;
    normalize_device_name(&hostname)
}

pub(crate) fn normalize_device_name(raw: &str) -> Option<String> {
    let compact = raw
        .chars()
        .fold(String::with_capacity(raw.len()), |mut acc, ch| {
            if ch.is_whitespace() {
                acc.push(' ');
            } else if !ch.is_control() {
                acc.push(ch);
            }
            acc
        });
    let normalized = compact.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }

    let limited = trimmed
        .chars()
        .take(MAX_DEVICE_NAME_LEN)
        .collect::<String>();
    if limited.is_empty() {
        None
    } else {
        Some(limited)
    }
}

// ─── Sync status persistence ─────────────────────────────────

pub(crate) fn persist_sync_success_best_effort<S, F>(
    settings: &mut S,
    manifest_hash: String,
    etag: Option<String>,
    persist_fn: F,
) -> bool
where
    F: FnOnce(&mut S, String, Option<String>) -> Result<(), AppError>,
{
    match persist_fn(settings, manifest_hash, etag) {
        Ok(()) => true,
        Err(err) => {
            log::warn!("[Sync] Persist sync status failed, keep operation success: {err}");
            false
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(sha256: &str, size: u64) -> ArtifactMeta {
        ArtifactMeta {
            sha256: sha256.to_string(),
            size,
        }
    }

    #[test]
    fn snapshot_id_is_stable() {
        let mut artifacts = BTreeMap::new();
        artifacts.insert("db.sql".to_string(), artifact("abc123", 100));

        let id1 = compute_snapshot_id(&artifacts);
        let id2 = compute_snapshot_id(&artifacts);
        assert_eq!(id1, id2);
    }

    #[test]
    fn snapshot_id_changes_with_artifacts() {
        let mut a1 = BTreeMap::new();
        a1.insert("db.sql".to_string(), artifact("hash-a", 1));

        let mut a2 = BTreeMap::new();
        a2.insert("db.sql".to_string(), artifact("hash-b", 1));

        assert_ne!(compute_snapshot_id(&a1), compute_snapshot_id(&a2));
    }

    #[test]
    fn sha256_hex_is_correct() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn persist_best_effort_returns_true_on_success() {
        let mut dummy = ();
        let ok = persist_sync_success_best_effort(
            &mut dummy,
            "hash".to_string(),
            Some("etag".to_string()),
            |_settings, _hash, _etag| Ok(()),
        );
        assert!(ok);
    }

    #[test]
    fn persist_best_effort_returns_false_on_error() {
        let mut dummy = ();
        let ok = persist_sync_success_best_effort(
            &mut dummy,
            "hash".to_string(),
            None,
            |_settings, _hash, _etag| Err(AppError::Config("boom".to_string())),
        );
        assert!(!ok);
    }

    fn manifest_with(
        format: &str,
        version: u32,
        gateway_data_version: Option<u32>,
    ) -> SyncManifest {
        let mut artifacts = BTreeMap::new();
        artifacts.insert("db.sql".to_string(), artifact("abc", 1));
        let snapshot_id = compute_snapshot_id(&artifacts);
        SyncManifest {
            format: format.to_string(),
            version,
            db_compat_version: gateway_data_version,
            device_name: "My Windows Host".to_string(),
            created_at: "2026-02-12T00:00:00Z".to_string(),
            artifacts,
            snapshot_id,
        }
    }

    #[test]
    fn validate_manifest_compat_accepts_supported_manifest() {
        let manifest = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        assert!(validate_manifest_compat(&manifest, RemoteLayout::Current).is_ok());
    }

    #[test]
    fn validate_manifest_compat_rejects_wrong_format_and_legacy_v2() {
        let manifest = manifest_with("cc-switch-webdav-sync", 2, Some(GATEWAY_DATA_VERSION));
        assert!(validate_manifest_compat(&manifest, RemoteLayout::Current).is_err());
        assert!(validate_manifest_compat(&manifest, RemoteLayout::Legacy).is_err());
    }

    #[test]
    fn validate_manifest_compat_rejects_wrong_version() {
        let manifest = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION + 1,
            Some(GATEWAY_DATA_VERSION),
        );
        assert!(validate_manifest_compat(&manifest, RemoteLayout::Current).is_err());
    }

    #[test]
    fn validate_manifest_compat_rejects_wrong_or_missing_gateway_data_version() {
        let wrong = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION + 1),
        );
        assert!(validate_manifest_compat(&wrong, RemoteLayout::Current).is_err());
        let missing = manifest_with(PROTOCOL_FORMAT, PROTOCOL_VERSION, None);
        assert!(validate_manifest_compat(&missing, RemoteLayout::Current).is_err());
    }

    #[test]
    fn validate_manifest_compat_requires_exact_db_sql_artifact_set() {
        let mut extra = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        extra
            .artifacts
            .insert("skills.zip".to_string(), artifact("legacy", 1));
        extra.snapshot_id = compute_snapshot_id(&extra.artifacts);
        assert!(validate_manifest_compat(&extra, RemoteLayout::Current).is_err());

        let mut missing = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        missing.artifacts.clear();
        missing.snapshot_id = compute_snapshot_id(&missing.artifacts);
        assert!(validate_manifest_compat(&missing, RemoteLayout::Current).is_err());
    }

    #[test]
    fn validate_manifest_compat_recomputes_snapshot_id() {
        let mut manifest = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        manifest.snapshot_id = "tampered".to_string();
        assert!(validate_manifest_compat(&manifest, RemoteLayout::Current).is_err());
    }

    #[test]
    fn effective_db_compat_version_is_gateway_data_version_only() {
        let manifest = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        assert_eq!(
            effective_db_compat_version(&manifest, RemoteLayout::Current),
            Some(GATEWAY_DATA_VERSION)
        );
        assert_eq!(
            effective_db_compat_version(&manifest, RemoteLayout::Legacy),
            Some(GATEWAY_DATA_VERSION)
        );
    }

    #[test]
    fn normalize_device_name_returns_none_for_blank_input() {
        assert_eq!(normalize_device_name("   \n\t  "), None);
    }

    #[test]
    fn normalize_device_name_collapses_whitespace_and_drops_control_chars() {
        assert_eq!(
            normalize_device_name("  Mac\tBook \n Pro\u{0007} "),
            Some("Mac Book Pro".to_string())
        );
    }

    #[test]
    fn normalize_device_name_truncates_to_max_len() {
        let long = "a".repeat(80);
        assert_eq!(normalize_device_name(&long).map(|s| s.len()), Some(64));
    }

    #[test]
    fn manifest_serialization_uses_device_name_only() {
        let manifest = manifest_with(
            PROTOCOL_FORMAT,
            PROTOCOL_VERSION,
            Some(GATEWAY_DATA_VERSION),
        );
        let value = serde_json::to_value(&manifest).expect("serialize manifest");
        assert!(
            value.get("deviceName").is_some(),
            "manifest should contain deviceName"
        );
        assert_eq!(
            value.get("dbCompatVersion").and_then(|v| v.as_u64()),
            Some(GATEWAY_DATA_VERSION as u64)
        );
        assert!(
            value.get("deviceId").is_none(),
            "manifest should not contain deviceId"
        );
    }

    #[test]
    fn validate_artifact_size_limit_rejects_oversized_artifacts() {
        let err = validate_artifact_size_limit(REMOTE_DB_SQL, MAX_SYNC_ARTIFACT_BYTES + 1)
            .expect_err("artifact larger than limit should be rejected");
        assert!(
            err.to_string().contains("too large") || err.to_string().contains("超过"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_artifact_size_limit_accepts_limit_boundary() {
        assert!(validate_artifact_size_limit(REMOTE_DB_SQL, MAX_SYNC_ARTIFACT_BYTES).is_ok());
    }

    #[test]
    fn verify_artifact_rejects_size_mismatch() {
        let meta = artifact("abc123", 100);
        let bytes = vec![0u8; 50];
        let err = verify_artifact(&bytes, "test.bin", &meta)
            .expect_err("size mismatch should be rejected");
        assert!(
            err.to_string().contains("mismatch") || err.to_string().contains("不匹配"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn verify_artifact_rejects_hash_mismatch() {
        let meta = ArtifactMeta {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            size: 5,
        };
        let bytes = b"hello";
        let err = verify_artifact(bytes, "test.bin", &meta)
            .expect_err("hash mismatch should be rejected");
        assert!(
            err.to_string().contains("verification failed") || err.to_string().contains("校验失败"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn verify_artifact_accepts_matching_data() {
        let data = b"hello";
        let meta = ArtifactMeta {
            sha256: sha256_hex(data),
            size: data.len() as u64,
        };
        assert!(verify_artifact(data, "test.bin", &meta).is_ok());
    }
}
