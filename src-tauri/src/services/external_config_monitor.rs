//! 七模块外部配置检测的内存状态与稳定目标采集。
//!
//! 本批次只建立 C3 的数据平面：复用 C2 snapshot adapter registry 读取 live 目标，
//! 生成确定性指纹，并维护按应用隔离的内存 generation/expected/conflict 状态。
//! 不启动 worker，不发送事件，也不读写 `proxy_live_backup`。

#![allow(dead_code)]

use crate::app_config::AppType;
use crate::proxy::snapshot::{SnapshotManifest, SnapshotTarget};
use crate::services::proxy::ProxyService;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const FINGERPRINT_DOMAIN: &[u8] = b"ags-managed-targets-v1";

/// 受管目标的内容类型。当前七模块全部使用 `file_bytes`；保留 semantic 标记，
/// 使指纹在 snapshot 契约未来扩展时仍显式区分 kind。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagedTargetKind {
    FileBytes,
    SemanticJson,
}

impl ManagedTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::FileBytes => "file_bytes",
            Self::SemanticJson => "semantic_json",
        }
    }
}

/// 一个模块内的稳定逻辑目标及其当前原始内容。
///
/// `id` 来自 C2 adapter，不持久化绝对路径。`existed=false` 与“存在但为空文件”
/// 必须严格区分，因此存在性独立于 `bytes` 参与指纹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedTarget {
    pub(crate) id: String,
    pub(crate) kind: ManagedTargetKind,
    pub(crate) existed: bool,
    pub(crate) bytes: Vec<u8>,
}

impl ManagedTarget {
    pub(crate) fn file_bytes(id: impl Into<String>, bytes: Option<&[u8]>) -> Self {
        Self {
            id: id.into(),
            kind: ManagedTargetKind::FileBytes,
            existed: bytes.is_some(),
            bytes: bytes.unwrap_or_default().to_vec(),
        }
    }

    fn from_snapshot_target(target: SnapshotTarget) -> Result<Self, String> {
        match target {
            SnapshotTarget::FileBytes {
                id,
                existed,
                payload_base64,
            } => {
                let snapshot_target = SnapshotTarget::FileBytes {
                    id: id.clone(),
                    existed,
                    payload_base64,
                };
                let bytes = snapshot_target.file_payload()?.unwrap_or_default();
                Ok(Self {
                    id,
                    kind: ManagedTargetKind::FileBytes,
                    existed,
                    bytes,
                })
            }
            SnapshotTarget::SemanticJson {
                id,
                existed,
                payload,
            } => {
                let bytes = match (existed, payload) {
                    (true, Some(value)) => serde_json::to_vec(&value)
                        .map_err(|error| format!("序列化 semantic_json 目标 {id} 失败: {error}"))?,
                    (false, None) => Vec::new(),
                    (true, None) => {
                        return Err(format!("存在的 semantic_json 目标 {id} 缺少 payload"));
                    }
                    (false, Some(_)) => {
                        return Err(format!("不存在的 semantic_json 目标 {id} 不得携带 payload"));
                    }
                };
                Ok(Self {
                    id,
                    kind: ManagedTargetKind::SemanticJson,
                    existed,
                    bytes,
                })
            }
        }
    }
}

/// AGS 当前预期受管内容。它只驻留内存，与首次接管的 restore snapshot 完全分离。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedExpected {
    pub(crate) generation: u64,
    pub(crate) targets: Vec<ManagedTarget>,
    pub(crate) fingerprint: String,
}

impl ManagedExpected {
    pub(crate) fn new(generation: u64, mut targets: Vec<ManagedTarget>) -> Result<Self, String> {
        if targets.is_empty() {
            return Err("受管目标至少需要一个条目".to_string());
        }
        targets.sort_by(|left, right| left.id.cmp(&right.id));

        for target in &targets {
            if target.id.trim().is_empty() {
                return Err("受管目标 id 不能为空".to_string());
            }
            if !target.existed && !target.bytes.is_empty() {
                return Err(format!("不存在的受管目标 {} 不得携带内容", target.id));
            }
        }
        for pair in targets.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(format!("受管目标 id 重复: {}", pair[0].id));
            }
        }

        let fingerprint = fingerprint_targets(&targets)?;
        Ok(Self {
            generation,
            targets,
            fingerprint,
        })
    }

    fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }
}

/// 已接管模块的显式外部冲突。更新 observed 时始终保留首次冲突前的 expected。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalConflict {
    pub(crate) generation: u64,
    pub(crate) expected: ManagedExpected,
    pub(crate) observed: ManagedExpected,
    pub(crate) detected_at_ms: i64,
}

/// 单模块外部配置状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModuleExternalState {
    pub(crate) generation: u64,
    pub(crate) expected: Option<ManagedExpected>,
    pub(crate) last_observed: Option<ManagedExpected>,
    pub(crate) conflict: Option<ExternalConflict>,
    pub(crate) managed_write_generation: Option<u64>,
}

/// 按规范 app_type 分区的 C3 内存状态容器。
///
/// `clear` 不删除 map entry，而是推进 generation 后清空内容，避免同一应用重新初始化时
/// generation 回退并让旧 UI 操作重新变得有效。
#[derive(Default)]
pub(crate) struct ExternalConfigStateStore {
    states: RwLock<HashMap<String, ModuleExternalState>>,
}

impl ExternalConfigStateStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn module_state(&self, app_type: &AppType) -> ModuleExternalState {
        self.states
            .read()
            .await
            .get(app_type.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// 初始化已接管模块的 expected baseline，同时把同一 capture 记为 last observed。
    pub(crate) async fn initialize_expected(
        &self,
        app_type: &AppType,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        let mut expected = ManagedExpected::new(0, targets)?;
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        let generation = advance_generation(state)?;
        expected.set_generation(generation);

        state.expected = Some(expected.clone());
        state.last_observed = Some(expected.clone());
        state.conflict = None;
        state.managed_write_generation = None;
        Ok(expected)
    }

    /// 记录未接管模块的外部变化。该路径只更新展示基线，不建立 managed ownership。
    pub(crate) async fn observe_unmanaged_change(
        &self,
        app_type: &AppType,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        let mut observed = ManagedExpected::new(0, targets)?;
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        let generation = advance_generation(state)?;
        observed.set_generation(generation);

        state.expected = None;
        state.last_observed = Some(observed.clone());
        state.conflict = None;
        state.managed_write_generation = None;
        Ok(observed)
    }

    /// 开始一次 AGS 受管写入并返回该写入的 generation token。
    pub(crate) async fn begin_managed_write(&self, app_type: &AppType) -> Result<u64, String> {
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        if let Some(active_generation) = state.managed_write_generation {
            return Err(format!(
                "{} 已有 generation {active_generation} 的受管写入进行中",
                app_type.as_str()
            ));
        }

        let generation = advance_generation(state)?;
        state.managed_write_generation = Some(generation);
        Ok(generation)
    }

    /// 完成对应 generation 的受管写入，以写后稳定 capture 更新 expected。
    pub(crate) async fn finish_managed_write(
        &self,
        app_type: &AppType,
        write_generation: u64,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        let mut expected = ManagedExpected::new(write_generation, targets)?;
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        if state.managed_write_generation != Some(write_generation)
            || state.generation != write_generation
        {
            return Err(format!(
                "{} 受管写入 generation 已过期：请求 {write_generation}，当前 {}",
                app_type.as_str(),
                state.generation
            ));
        }

        expected.set_generation(write_generation);
        state.expected = Some(expected.clone());
        state.last_observed = Some(expected.clone());
        state.conflict = None;
        state.managed_write_generation = None;
        Ok(expected)
    }

    /// 创建或更新冲突。后续外变只替换 observed 和 generation，首次 expected 不变。
    pub(crate) async fn create_or_update_conflict(
        &self,
        app_type: &AppType,
        targets: Vec<ManagedTarget>,
    ) -> Result<ExternalConflict, String> {
        let mut observed = ManagedExpected::new(0, targets)?;
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        if let Some(active_generation) = state.managed_write_generation {
            return Err(format!(
                "{} generation {active_generation} 的受管写入尚未结束，暂不判定外部冲突",
                app_type.as_str()
            ));
        }

        let expected = state
            .conflict
            .as_ref()
            .map(|conflict| conflict.expected.clone())
            .or_else(|| state.expected.clone())
            .ok_or_else(|| format!("{} 尚未初始化 managed expected", app_type.as_str()))?;
        let generation = advance_generation(state)?;
        observed.set_generation(generation);

        let conflict = ExternalConflict {
            generation,
            expected,
            observed: observed.clone(),
            detected_at_ms: current_time_ms(),
        };
        state.last_observed = Some(observed);
        state.conflict = Some(conflict.clone());
        Ok(conflict)
    }

    /// 清除该模块的 expected/observed/conflict/in-flight 状态，但保留单调 generation。
    pub(crate) async fn clear(&self, app_type: &AppType) -> Result<u64, String> {
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        let generation = advance_generation(state)?;
        state.expected = None;
        state.last_observed = None;
        state.conflict = None;
        state.managed_write_generation = None;
        Ok(generation)
    }
}

/// 只调用 C2 adapter 的内存 capture；不会调用 `capture_snapshot_once`，也不会访问 DAO。
/// 返回目标按稳定 id 排序，供后续 worker 在已有 per-app switch lock 内进行双次稳定采集。
pub(crate) fn capture_managed_expected(
    app_type: &AppType,
    generation: u64,
) -> Result<ManagedExpected, String> {
    let adapter = ProxyService::snapshot_adapter_for_app(app_type)?
        .ok_or_else(|| format!("{} 缺少 snapshot adapter", app_type.as_str()))?;
    if adapter.app_type() != *app_type {
        return Err(format!(
            "snapshot adapter app_type 不匹配：期望 {}，实际 {}",
            app_type.as_str(),
            adapter.app_type().as_str()
        ));
    }

    // 用现有 manifest 校验 target id 唯一性、kind/existed/payload 组合，但不编码或持久化。
    let manifest = SnapshotManifest::new(app_type, adapter.capture_targets()?)?;
    let targets = manifest
        .targets
        .into_iter()
        .map(ManagedTarget::from_snapshot_target)
        .collect::<Result<Vec<_>, _>>()?;
    ManagedExpected::new(generation, targets)
}

fn fingerprint_targets(targets: &[ManagedTarget]) -> Result<String, String> {
    let mut hasher = Sha256::new();
    update_length_prefixed(&mut hasher, FINGERPRINT_DOMAIN)?;

    for target in targets {
        update_length_prefixed(&mut hasher, target.id.as_bytes())?;
        update_length_prefixed(&mut hasher, target.kind.as_str().as_bytes())?;
        hasher.update([u8::from(target.existed)]);
        update_length_prefixed(&mut hasher, &target.bytes)?;
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len()).map_err(|_| "受管目标内容长度超出 u64".to_string())?;
    hasher.update(length.to_be_bytes());
    hasher.update(bytes);
    Ok(())
}

fn advance_generation(state: &mut ModuleExternalState) -> Result<u64, String> {
    let generation = state
        .generation
        .checked_add(1)
        .ok_or_else(|| "外部配置 generation 已耗尽".to_string())?;
    state.generation = generation;
    Ok(generation)
}

fn current_time_ms() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    millis.min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    struct TempHome {
        dir: TempDir,
        original_env: Vec<(&'static str, Option<OsString>)>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("创建临时 HOME");
            let keys = [
                "HOME",
                "USERPROFILE",
                "AGENT_SWITCH_TEST_HOME",
                "LOCALAPPDATA",
                "APPDATA",
                "HERMES_HOME",
                "OPENCODE_DB",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
            ];
            let original_env = keys
                .into_iter()
                .map(|key| (key, env::var_os(key)))
                .collect();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("AGENT_SWITCH_TEST_HOME", dir.path());
            env::set_var("LOCALAPPDATA", dir.path().join("AppData").join("Local"));
            env::set_var("APPDATA", dir.path().join("AppData").join("Roaming"));
            env::set_var("HERMES_HOME", dir.path().join("hermes"));
            env::set_var("OPENCODE_DB", dir.path().join("opencode.db"));
            env::set_var("XDG_CONFIG_HOME", dir.path().join(".config"));
            env::set_var("XDG_DATA_HOME", dir.path().join(".local").join("share"));

            Self { dir, original_env }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            for (key, value) in &self.original_env {
                match value {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
        }
    }

    fn bytes_target(id: &str, bytes: &[u8]) -> ManagedTarget {
        ManagedTarget::file_bytes(id, Some(bytes))
    }

    #[test]
    fn fingerprint_is_deterministic_over_target_order() {
        let first = ManagedExpected::new(
            1,
            vec![bytes_target("z", b"last"), bytes_target("a", b"first")],
        )
        .unwrap();
        let second = ManagedExpected::new(
            2,
            vec![bytes_target("a", b"first"), bytes_target("z", b"last")],
        )
        .unwrap();

        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            first
                .targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z"]
        );
    }

    #[test]
    fn fingerprint_changes_for_bytes_and_existence() {
        let original = ManagedExpected::new(1, vec![bytes_target("settings", b"one")]).unwrap();
        let changed = ManagedExpected::new(2, vec![bytes_target("settings", b"two")]).unwrap();
        let existing_empty = ManagedExpected::new(3, vec![bytes_target("settings", b"")]).unwrap();
        let missing =
            ManagedExpected::new(4, vec![ManagedTarget::file_bytes("settings", None)]).unwrap();
        let semantic_same_bytes = ManagedExpected::new(
            5,
            vec![ManagedTarget {
                id: "settings".to_string(),
                kind: ManagedTargetKind::SemanticJson,
                existed: true,
                bytes: b"one".to_vec(),
            }],
        )
        .unwrap();

        assert_ne!(original.fingerprint, changed.fingerprint);
        assert_ne!(existing_empty.fingerprint, missing.fingerprint);
        assert_ne!(original.fingerprint, semantic_same_bytes.fingerprint);
    }

    #[test]
    fn fingerprint_and_capture_support_non_utf8() {
        let binary = [0, 0x80, 0xff, b'\n'];
        let expected = ManagedExpected::new(1, vec![bytes_target("config.yaml", &binary)]).unwrap();

        assert_eq!(expected.targets[0].bytes, binary);
        assert_eq!(expected.fingerprint.len(), 64);
    }

    #[test]
    #[serial]
    fn dispatcher_covers_all_seven_modules_with_stable_target_ids() {
        let _home = TempHome::new();
        let cases = [
            (AppType::Claude, vec!["settings"]),
            (
                AppType::ClaudeDesktop,
                vec!["meta", "normal_config", "profile", "threep_config"],
            ),
            (AppType::Codex, vec!["auth", "config", "model_catalog"]),
            (AppType::Gemini, vec![".env"]),
            (AppType::OpenCode, vec!["opencode.json"]),
            (AppType::OpenClaw, vec!["openclaw.json"]),
            (AppType::Hermes, vec!["config.yaml"]),
        ];

        for (app_type, expected_ids) in cases {
            let capture = capture_managed_expected(&app_type, 7)
                .unwrap_or_else(|error| panic!("capture {} 失败: {error}", app_type.as_str()));
            let actual_ids = capture
                .targets
                .iter()
                .map(|target| target.id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual_ids, expected_ids, "{} target ids", app_type.as_str());
            assert!(capture
                .targets
                .iter()
                .all(|target| target.kind == ManagedTargetKind::FileBytes));
        }
    }

    #[test]
    #[serial]
    fn opencode_capture_only_reads_opencode_json_and_leaves_database_untouched() {
        let home = TempHome::new();
        let config_path = crate::opencode_config::get_opencode_config_path();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let config_bytes = [b'{', b'}', b'\n', 0xff];
        fs::write(&config_path, config_bytes).unwrap();

        let database_path = home.path().join("opencode.db");
        let database_bytes = [0, 1, 2, 0xfe, 0xff];
        fs::write(&database_path, database_bytes).unwrap();
        let modified_before = fs::metadata(&database_path).unwrap().modified().unwrap();

        let capture = capture_managed_expected(&AppType::OpenCode, 1).unwrap();

        assert_eq!(capture.targets.len(), 1);
        assert_eq!(capture.targets[0].id, "opencode.json");
        assert_eq!(capture.targets[0].bytes, config_bytes);
        assert_eq!(fs::read(&database_path).unwrap(), database_bytes);
        assert_eq!(
            fs::metadata(&database_path).unwrap().modified().unwrap(),
            modified_before
        );
    }

    #[tokio::test]
    async fn generation_is_monotonic_across_state_transitions() {
        let store = ExternalConfigStateStore::new();
        let initialized = store
            .initialize_expected(&AppType::Claude, vec![bytes_target("settings", b"managed")])
            .await
            .unwrap();
        let write_generation = store.begin_managed_write(&AppType::Claude).await.unwrap();
        let finished = store
            .finish_managed_write(
                &AppType::Claude,
                write_generation,
                vec![bytes_target("settings", b"rewritten")],
            )
            .await
            .unwrap();
        let conflict = store
            .create_or_update_conflict(
                &AppType::Claude,
                vec![bytes_target("settings", b"external")],
            )
            .await
            .unwrap();
        let cleared = store.clear(&AppType::Claude).await.unwrap();
        let unmanaged = store
            .observe_unmanaged_change(
                &AppType::Claude,
                vec![bytes_target("settings", b"unmanaged")],
            )
            .await
            .unwrap();

        assert!(initialized.generation < write_generation);
        assert_eq!(finished.generation, write_generation);
        assert!(write_generation < conflict.generation);
        assert!(conflict.generation < cleared);
        assert!(cleared < unmanaged.generation);
        assert_eq!(
            store.module_state(&AppType::Claude).await.generation,
            unmanaged.generation
        );
    }

    #[tokio::test]
    async fn conflict_updates_observed_but_preserves_first_expected() {
        let store = ExternalConfigStateStore::new();
        let expected = store
            .initialize_expected(
                &AppType::Hermes,
                vec![bytes_target("config.yaml", b"managed")],
            )
            .await
            .unwrap();
        let first = store
            .create_or_update_conflict(
                &AppType::Hermes,
                vec![bytes_target("config.yaml", b"external-1")],
            )
            .await
            .unwrap();
        let second = store
            .create_or_update_conflict(
                &AppType::Hermes,
                vec![bytes_target("config.yaml", b"external-2")],
            )
            .await
            .unwrap();

        assert_eq!(first.expected, expected);
        assert_eq!(second.expected, expected);
        assert_ne!(first.observed.fingerprint, second.observed.fingerprint);
        assert!(first.generation < second.generation);
        assert_eq!(second.observed.generation, second.generation);

        let state = store.module_state(&AppType::Hermes).await;
        assert_eq!(state.expected, Some(expected));
        assert_eq!(state.conflict, Some(second));
    }
}
