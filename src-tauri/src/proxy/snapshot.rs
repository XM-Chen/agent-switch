//! 七模块共用的版本化精确快照契约。
//!
//! 模块路径解析、SQLite 事务和多目标补偿由 C2 的模块 adapter 实现；本模块只负责
//! manifest 编解码、一次性 capture、legacy 分流，以及恢复成功后的原子所有权清理。
//!
//! C1 先落地契约与测试；C2a/C2b 再接入模块 adapter 消费方。

#![allow(dead_code)]

use crate::app_config::AppType;
use crate::database::Database;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::str::FromStr;

pub const SNAPSHOT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotManifest {
    pub version: u32,
    pub app_type: String,
    pub targets: Vec<SnapshotTarget>,
}

impl SnapshotManifest {
    pub fn new(app_type: &AppType, targets: Vec<SnapshotTarget>) -> Result<Self, String> {
        let manifest = Self {
            version: SNAPSHOT_MANIFEST_VERSION,
            app_type: app_type.as_str().to_string(),
            targets,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != SNAPSHOT_MANIFEST_VERSION {
            return Err(format!(
                "不支持的快照 manifest 版本 {}（当前支持 {}）",
                self.version, SNAPSHOT_MANIFEST_VERSION
            ));
        }
        let app = AppType::from_str(&self.app_type)
            .map_err(|error| format!("快照 app_type 无效: {error}"))?;
        if app.as_str() != self.app_type {
            return Err("快照 app_type 必须使用 AppType::as_str() 的规范值".to_string());
        }
        if self.targets.is_empty() {
            return Err("快照 manifest 至少需要一个目标".to_string());
        }

        let mut ids = HashSet::new();
        for target in &self.targets {
            let id = target.id();
            if id.trim().is_empty() {
                return Err("快照目标 id 不能为空".to_string());
            }
            if !ids.insert(id) {
                return Err(format!("快照目标 id 重复: {id}"));
            }
            target.validate()?;
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| format!("序列化快照 manifest 失败: {error}"))
    }

    pub fn decode(raw: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(raw)
            .map_err(|error| format!("解析快照 manifest 失败: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotTarget {
    FileBytes {
        id: String,
        existed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload_base64: Option<String>,
    },
    SemanticJson {
        id: String,
        existed: bool,
        payload: Option<Value>,
    },
}

impl SnapshotTarget {
    pub fn file_bytes(id: impl Into<String>, bytes: Option<&[u8]>) -> Self {
        Self::FileBytes {
            id: id.into(),
            existed: bytes.is_some(),
            payload_base64: bytes.map(|value| BASE64.encode(value)),
        }
    }

    pub fn semantic_json(id: impl Into<String>, payload: Option<Value>) -> Self {
        Self::SemanticJson {
            id: id.into(),
            existed: payload.is_some(),
            payload,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::FileBytes { id, .. } | Self::SemanticJson { id, .. } => id,
        }
    }

    pub fn file_payload(&self) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::FileBytes {
                existed,
                payload_base64,
                ..
            } => {
                if !existed {
                    return Ok(None);
                }
                let payload = payload_base64
                    .as_deref()
                    .ok_or_else(|| "存在的 file_bytes 目标缺少 payload_base64".to_string())?;
                BASE64
                    .decode(payload)
                    .map(Some)
                    .map_err(|error| format!("解码 file_bytes payload 失败: {error}"))
            }
            Self::SemanticJson { .. } => Err("目标不是 file_bytes".to_string()),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::FileBytes {
                existed,
                payload_base64,
                ..
            } => match (*existed, payload_base64) {
                (true, Some(payload)) => BASE64
                    .decode(payload)
                    .map(|_| ())
                    .map_err(|error| format!("file_bytes payload_base64 无效: {error}")),
                (true, None) => Err("存在的 file_bytes 目标缺少 payload_base64".to_string()),
                (false, Some(_)) => Err("不存在的 file_bytes 目标不得携带 payload".to_string()),
                (false, None) => Ok(()),
            },
            Self::SemanticJson {
                existed, payload, ..
            } => match (*existed, payload) {
                (true, Some(_)) | (false, None) => Ok(()),
                (true, None) => Err("存在的 semantic_json 目标缺少 payload".to_string()),
                (false, Some(_)) => Err("不存在的 semantic_json 目标不得携带 payload".to_string()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacySnapshot {
    pub app_type: String,
    pub original_config: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedSnapshot {
    Manifest(SnapshotManifest),
    Legacy(LegacySnapshot),
}

/// 识别版本化 manifest；无 version 的三旧模块 JSON 交给 legacy adapter。
pub fn decode_stored_snapshot(app_type: &str, raw: &str) -> Result<DecodedSnapshot, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("快照不是有效 JSON: {error}"))?;
    if value.get("version").is_some() {
        let manifest = SnapshotManifest::decode(raw)?;
        if manifest.app_type != app_type {
            return Err(format!(
                "快照 app_type 不匹配：期望 {app_type}，实际 {}",
                manifest.app_type
            ));
        }
        return Ok(DecodedSnapshot::Manifest(manifest));
    }

    if !matches!(app_type, "claude" | "codex" | "gemini") {
        return Err(format!("{app_type} 不支持旧版无版本快照"));
    }
    Ok(DecodedSnapshot::Legacy(LegacySnapshot {
        app_type: app_type.to_string(),
        original_config: raw.to_string(),
    }))
}

/// 模块 adapter 契约。多目标恢复失败后的补偿回滚由实现方在
/// `restore_manifest_transactional` 内完成。
pub trait SnapshotModuleAdapter: Send + Sync {
    fn app_type(&self) -> AppType;
    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String>;
    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String>;
    fn restore_legacy(&self, legacy: &LegacySnapshot) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSnapshotOutcome {
    Captured,
    PreservedExisting,
}

/// 仅第一次 capture 写入；已有有效 manifest/legacy 快照时绝不覆盖。
pub async fn capture_snapshot_once(
    db: &Database,
    adapter: &dyn SnapshotModuleAdapter,
) -> Result<CaptureSnapshotOutcome, String> {
    let app_type = adapter.app_type();
    if let Some(existing) = db
        .get_live_backup(app_type.as_str())
        .await
        .map_err(|error| format!("读取已有快照失败: {error}"))?
    {
        decode_stored_snapshot(app_type.as_str(), &existing.original_config)?;
        return Ok(CaptureSnapshotOutcome::PreservedExisting);
    }

    let manifest = SnapshotManifest::new(&app_type, adapter.capture_targets()?)?;
    let encoded = manifest.encode()?;
    let inserted = db
        .insert_live_snapshot_if_absent(app_type.as_str(), &encoded)
        .await
        .map_err(|error| format!("保存快照失败: {error}"))?;
    Ok(if inserted {
        CaptureSnapshotOutcome::Captured
    } else {
        CaptureSnapshotOutcome::PreservedExisting
    })
}

/// 只有 adapter 完整恢复成功后，才原子清 enabled 与快照；失败时两者均保留。
pub async fn restore_snapshot_and_release(
    db: &Database,
    adapter: &dyn SnapshotModuleAdapter,
) -> Result<(), String> {
    let app_type = adapter.app_type();
    let backup = db
        .get_live_backup(app_type.as_str())
        .await
        .map_err(|error| format!("读取快照失败: {error}"))?
        .ok_or_else(|| format!("{} 没有可恢复快照", app_type.as_str()))?;

    match decode_stored_snapshot(app_type.as_str(), &backup.original_config)? {
        DecodedSnapshot::Manifest(manifest) => {
            adapter.restore_manifest_transactional(&manifest)?;
        }
        DecodedSnapshot::Legacy(legacy) => adapter.restore_legacy(&legacy)?,
    }

    db.release_takeover_ownership(app_type.as_str())
        .await
        .map_err(|error| format!("清理接管所有权失败: {error}"))
}

/// direct 模式放弃所有权：不写 Live，只原子清状态与快照。
pub async fn abandon_snapshot_ownership(db: &Database, app_type: &AppType) -> Result<(), String> {
    db.release_takeover_ownership(app_type.as_str())
        .await
        .map_err(|error| format!("放弃接管所有权失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::types::RouteMode;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct FakeAdapter {
        app: AppType,
        targets: Vec<SnapshotTarget>,
        fail_restore: AtomicBool,
    }

    impl SnapshotModuleAdapter for FakeAdapter {
        fn app_type(&self) -> AppType {
            self.app.clone()
        }

        fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
            Ok(self.targets.clone())
        }

        fn restore_manifest_transactional(
            &self,
            _manifest: &SnapshotManifest,
        ) -> Result<(), String> {
            if self.fail_restore.load(Ordering::SeqCst) {
                Err("injected restore failure".to_string())
            } else {
                Ok(())
            }
        }

        fn restore_legacy(&self, _legacy: &LegacySnapshot) -> Result<(), String> {
            self.restore_manifest_transactional(&SnapshotManifest::new(
                &self.app,
                self.targets.clone(),
            )?)
        }
    }

    #[test]
    fn manifest_round_trips_binary_missing_multi_target_and_semantic_payloads() {
        let binary = [0, 159, 255, b'\n', b'\r'];
        let manifest = SnapshotManifest::new(
            &AppType::OpenCode,
            vec![
                SnapshotTarget::file_bytes("settings", Some(&binary)),
                SnapshotTarget::file_bytes("auth", None),
                SnapshotTarget::semantic_json(
                    "sqlite_accounts",
                    Some(serde_json::json!({"rows": [[1, "token"]]})),
                ),
            ],
        )
        .expect("build manifest");

        let encoded = manifest.encode().expect("encode manifest");
        let decoded = SnapshotManifest::decode(&encoded).expect("decode manifest");
        assert_eq!(decoded, manifest);
        assert_eq!(
            decoded.targets[0].file_payload().unwrap(),
            Some(binary.to_vec())
        );
        assert_eq!(decoded.targets[1].file_payload().unwrap(), None);
    }

    #[test]
    fn legacy_decoder_only_accepts_three_original_modules() {
        assert!(matches!(
            decode_stored_snapshot("claude", r#"{"env":{"A":"B"}}"#).unwrap(),
            DecodedSnapshot::Legacy(_)
        ));
        assert!(decode_stored_snapshot("hermes", "{}").is_err());
    }

    #[tokio::test]
    async fn capture_once_preserves_existing_snapshot() {
        let db = Database::memory().expect("memory db");
        let first = FakeAdapter {
            app: AppType::OpenCode,
            targets: vec![SnapshotTarget::file_bytes("settings", Some(&[0, 255]))],
            fail_restore: AtomicBool::new(false),
        };
        assert_eq!(
            capture_snapshot_once(&db, &first).await.unwrap(),
            CaptureSnapshotOutcome::Captured
        );
        let original = db
            .get_live_backup("opencode")
            .await
            .unwrap()
            .unwrap()
            .original_config;

        let second = FakeAdapter {
            app: AppType::OpenCode,
            targets: vec![SnapshotTarget::file_bytes("settings", Some(b"changed"))],
            fail_restore: AtomicBool::new(false),
        };
        assert_eq!(
            capture_snapshot_once(&db, &second).await.unwrap(),
            CaptureSnapshotOutcome::PreservedExisting
        );
        assert_eq!(
            db.get_live_backup("opencode")
                .await
                .unwrap()
                .unwrap()
                .original_config,
            original
        );
    }

    #[tokio::test]
    async fn failed_restore_keeps_state_and_snapshot_success_releases_both() {
        let db = Database::memory().expect("memory db");
        let adapter = FakeAdapter {
            app: AppType::OpenCode,
            targets: vec![SnapshotTarget::semantic_json(
                "sqlite_accounts",
                Some(serde_json::json!({"rows": []})),
            )],
            fail_restore: AtomicBool::new(true),
        };
        capture_snapshot_once(&db, &adapter).await.unwrap();
        let mut config = db.get_proxy_config_for_app("opencode").await.unwrap();
        config.takeover_enabled = true;
        config.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(config).await.unwrap();

        assert!(restore_snapshot_and_release(&db, &adapter).await.is_err());
        assert!(
            db.get_proxy_config_for_app("opencode")
                .await
                .unwrap()
                .takeover_enabled
        );
        assert!(db.get_live_backup("opencode").await.unwrap().is_some());

        adapter.fail_restore.store(false, Ordering::SeqCst);
        restore_snapshot_and_release(&db, &adapter).await.unwrap();
        assert!(
            !db.get_proxy_config_for_app("opencode")
                .await
                .unwrap()
                .takeover_enabled
        );
        assert!(db.get_live_backup("opencode").await.unwrap().is_none());
    }
}
