//! 七模块外部配置检测服务。
//!
//! 复用 C2 snapshot adapter registry 与 per-app switch lock，使用标准库 metadata 快速探测、
//! 稳定全文采集和内存 generation/conflict 状态检测外部变化。本模块不读写
//! `proxy_live_backup`，也不会由后台任务改写 live/provider/current/route DB。

#![allow(dead_code)]

use crate::app_config::AppType;
use crate::database::Database;
use crate::proxy::snapshot::{SnapshotManifest, SnapshotTarget};
use crate::proxy::types::RouteMode;
use crate::services::proxy::ProxyService;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::Emitter;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinHandle;

pub const EXTERNAL_CONFIG_CHANGED_EVENT: &str = "external-config-changed";
const FINGERPRINT_DOMAIN: &[u8] = b"ags-managed-targets-v1";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_DEBOUNCE_INTERVAL: Duration = Duration::from_millis(200);
const DEFAULT_FULL_SCAN_INTERVAL: Duration = Duration::from_secs(5);

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

    /// 初始化未接管模块的只读 observed baseline，不发事件，也不建立 managed ownership。
    pub(crate) async fn initialize_unmanaged_baseline(
        &self,
        app_type: &AppType,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        let ownership_changed = state.expected.is_some()
            || state.conflict.is_some()
            || state.managed_write_generation.is_some();
        if ownership_changed {
            advance_generation(state)?;
        }

        let observed = ManagedExpected::new(state.generation, targets)?;
        state.expected = None;
        state.last_observed = Some(observed.clone());
        state.conflict = None;
        state.managed_write_generation = None;
        Ok(observed)
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

    /// 记录“全文重新等于 expected”的稳定 capture。已有冲突不会被后台自动清除。
    pub(crate) async fn observe_expected_equal(
        &self,
        app_type: &AppType,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        let mut states = self.states.write().await;
        let state = states.entry(app_type.as_str().to_string()).or_default();
        let expected = state
            .expected
            .as_ref()
            .ok_or_else(|| format!("{} 尚未初始化 managed expected", app_type.as_str()))?;
        let mut observed = ManagedExpected::new(state.generation, targets)?;
        if observed.fingerprint != expected.fingerprint {
            return Err(format!("{} observed 与 expected 不一致", app_type.as_str()));
        }
        observed.set_generation(state.generation);
        state.last_observed = Some(observed.clone());
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

/// 前端收到事件后按规范 app_type 使对应 live 查询失效。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalConfigChangedPayload {
    pub app_type: String,
    pub generation: u64,
    pub conflict: bool,
    pub takeover_enabled: bool,
}

/// 七模块只读状态查询项。冲突全文和 managed expected 不暴露到 wire。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalConfigModuleStatus {
    pub app_type: String,
    pub generation: u64,
    pub conflict: bool,
    pub takeover_enabled: bool,
    pub route_mode: RouteMode,
}

trait ExternalConfigEventSink: Send + Sync {
    fn emit_changed(&self, payload: &ExternalConfigChangedPayload) -> Result<(), String>;
}

struct AppHandleEventSink(tauri::AppHandle);

impl ExternalConfigEventSink for AppHandleEventSink {
    fn emit_changed(&self, payload: &ExternalConfigChangedPayload) -> Result<(), String> {
        self.0
            .emit(EXTERNAL_CONFIG_CHANGED_EVENT, payload)
            .map_err(|error| format!("发送 {EXTERNAL_CONFIG_CHANGED_EVENT} 事件失败: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuickTargetMetadata {
    path: PathBuf,
    existed: bool,
    len: u64,
    modified_ns: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuickMetadataSignature {
    targets: Vec<QuickTargetMetadata>,
}

trait MonitorCaptureSource: Send + Sync {
    fn quick_signature(&self, app_type: &AppType) -> Result<QuickMetadataSignature, String>;
    fn capture(&self, app_type: &AppType) -> Result<ManagedExpected, String>;
}

struct SystemMonitorCaptureSource;

impl MonitorCaptureSource for SystemMonitorCaptureSource {
    fn quick_signature(&self, app_type: &AppType) -> Result<QuickMetadataSignature, String> {
        let targets = monitored_target_paths(app_type)?
            .into_iter()
            .map(|path| match std::fs::metadata(&path) {
                Ok(metadata) => {
                    let modified_ns = metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                        .map(|duration| duration.as_nanos());
                    Ok(QuickTargetMetadata {
                        path,
                        existed: true,
                        len: metadata.len(),
                        modified_ns,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(QuickTargetMetadata {
                        path,
                        existed: false,
                        len: 0,
                        modified_ns: None,
                    })
                }
                Err(error) => Err(format!("读取 {} 元数据失败: {error}", path.display())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(QuickMetadataSignature { targets })
    }

    fn capture(&self, app_type: &AppType) -> Result<ManagedExpected, String> {
        capture_managed_expected(app_type, 0)
    }
}

#[derive(Debug, Clone, Copy)]
struct MonitorOptions {
    poll_interval: Duration,
    debounce_interval: Duration,
    full_scan_interval: Duration,
}

impl Default for MonitorOptions {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            debounce_interval: DEFAULT_DEBOUNCE_INTERVAL,
            full_scan_interval: DEFAULT_FULL_SCAN_INTERVAL,
        }
    }
}

#[derive(Default)]
struct MonitorWorker {
    started_once: bool,
    stop_tx: Option<oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ModulePollTracker {
    committed_quick: Option<QuickMetadataSignature>,
    committed_takeover_enabled: Option<bool>,
    candidate: Option<StableCandidate>,
    last_full_scan: Option<Instant>,
}

struct StableCandidate {
    capture: ManagedExpected,
    first_seen: Instant,
    confirmations: u8,
}

#[derive(Clone)]
struct MonitorRuntime {
    db: Arc<Database>,
    proxy_service: ProxyService,
    state_store: Arc<ExternalConfigStateStore>,
    event_sink: Arc<StdRwLock<Option<Arc<dyn ExternalConfigEventSink>>>>,
    capture_source: Arc<dyn MonitorCaptureSource>,
    options: MonitorOptions,
}

impl MonitorRuntime {
    async fn run(self, mut stop_rx: oneshot::Receiver<()>) {
        let mut trackers: HashMap<String, ModulePollTracker> = AppType::all()
            .map(|app_type| (app_type.as_str().to_string(), ModulePollTracker::default()))
            .collect();
        let mut interval = tokio::time::interval(self.options.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = &mut stop_rx => break,
                _ = interval.tick() => {
                    for app_type in AppType::all() {
                        let tracker = trackers
                            .get_mut(app_type.as_str())
                            .expect("七模块 tracker 已预先建立");
                        if let Err(error) = self.poll_module(&app_type, tracker).await {
                            tracker.candidate = None;
                            log::warn!(
                                "[ExternalConfigMonitor] {} 轮询失败，将在下一轮重试: {error}",
                                app_type.as_str()
                            );
                        }
                    }
                }
            }
        }
    }

    async fn poll_module(
        &self,
        app_type: &AppType,
        tracker: &mut ModulePollTracker,
    ) -> Result<(), String> {
        let config = self
            .db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .map_err(|error| format!("读取接管状态失败: {error}"))?;
        let state_before = self.state_store.module_state(app_type).await;
        if state_before.managed_write_generation.is_some() {
            tracker.candidate = None;
            return Ok(());
        }

        let quick = self.capture_source.quick_signature(app_type)?;
        let now = Instant::now();
        let periodic_full_scan_due = tracker
            .last_full_scan
            .map(|last| now.duration_since(last) >= self.options.full_scan_interval)
            .unwrap_or(true);
        let quick_changed = tracker.committed_quick.as_ref() != Some(&quick);
        let ownership_changed = tracker.committed_takeover_enabled != Some(config.takeover_enabled);
        if !quick_changed
            && !ownership_changed
            && tracker.candidate.is_none()
            && !periodic_full_scan_due
        {
            return Ok(());
        }

        // 必须复用 C2 的 per-app switch lock；该锁内一次采集全部 target。
        let _switch_guard = self
            .proxy_service
            .lock_switch_for_app(app_type.as_str())
            .await;
        let state_locked = self.state_store.module_state(app_type).await;
        if state_locked.managed_write_generation.is_some() {
            tracker.candidate = None;
            return Ok(());
        }

        let capture = self.capture_source.capture(app_type)?;
        tracker.last_full_scan = Some(now);

        // 锁内二次读取 ownership，避免 capture 前后接管状态切换导致错误判定。
        let config = self
            .db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .map_err(|error| format!("二次读取接管状态失败: {error}"))?;
        let state = self.state_store.module_state(app_type).await;
        if state.managed_write_generation.is_some() {
            tracker.candidate = None;
            return Ok(());
        }

        let ownership_needs_baseline = if config.takeover_enabled {
            state.expected.is_none()
        } else {
            state.expected.is_some() || state.conflict.is_some()
        };
        let matches_last_observed = state
            .last_observed
            .as_ref()
            .map(|observed| observed.fingerprint.as_str())
            == Some(capture.fingerprint.as_str());
        if !ownership_needs_baseline && matches_last_observed {
            tracker.committed_quick = Some(quick);
            tracker.committed_takeover_enabled = Some(config.takeover_enabled);
            tracker.candidate = None;
            return Ok(());
        }

        let candidate = match tracker.candidate.as_mut() {
            Some(candidate) if candidate.capture.fingerprint == capture.fingerprint => {
                candidate.confirmations = candidate.confirmations.saturating_add(1);
                candidate
            }
            _ => {
                tracker.candidate = Some(StableCandidate {
                    capture,
                    first_seen: now,
                    confirmations: 1,
                });
                return Ok(());
            }
        };

        if candidate.confirmations < 2
            || now.duration_since(candidate.first_seen) < self.options.debounce_interval
        {
            return Ok(());
        }

        let stable_capture = tracker
            .candidate
            .take()
            .expect("candidate 已完成连续稳定确认")
            .capture;
        self.commit_stable_capture(app_type, &config, stable_capture)
            .await?;
        tracker.committed_quick = Some(quick);
        tracker.committed_takeover_enabled = Some(config.takeover_enabled);
        Ok(())
    }

    async fn commit_stable_capture(
        &self,
        app_type: &AppType,
        config: &crate::proxy::types::AppProxyConfig,
        capture: ManagedExpected,
    ) -> Result<(), String> {
        let state = self.state_store.module_state(app_type).await;
        if state.managed_write_generation.is_some() {
            return Ok(());
        }

        if !config.takeover_enabled {
            if state.last_observed.is_none() || state.expected.is_some() || state.conflict.is_some()
            {
                self.state_store
                    .initialize_unmanaged_baseline(app_type, capture.targets)
                    .await?;
                return Ok(());
            }
            if state
                .last_observed
                .as_ref()
                .is_some_and(|observed| observed.fingerprint == capture.fingerprint)
            {
                return Ok(());
            }

            let observed = self
                .state_store
                .observe_unmanaged_change(app_type, capture.targets)
                .await?;
            self.emit(ExternalConfigChangedPayload {
                app_type: app_type.as_str().to_string(),
                generation: observed.generation,
                conflict: false,
                takeover_enabled: false,
            });
            return Ok(());
        }

        let Some(expected) = state.expected else {
            self.state_store
                .initialize_expected(app_type, capture.targets)
                .await?;
            return Ok(());
        };
        if expected.fingerprint == capture.fingerprint {
            self.state_store
                .observe_expected_equal(app_type, capture.targets)
                .await?;
            return Ok(());
        }
        if state
            .last_observed
            .as_ref()
            .is_some_and(|observed| observed.fingerprint == capture.fingerprint)
        {
            return Ok(());
        }

        let conflict = self
            .state_store
            .create_or_update_conflict(app_type, capture.targets)
            .await?;
        self.emit(ExternalConfigChangedPayload {
            app_type: app_type.as_str().to_string(),
            generation: conflict.generation,
            conflict: true,
            takeover_enabled: true,
        });
        Ok(())
    }

    fn emit(&self, payload: ExternalConfigChangedPayload) {
        let sink = match self.event_sink.read() {
            Ok(sink) => sink.clone(),
            Err(error) => {
                log::error!("[ExternalConfigMonitor] 事件 sink 锁损坏: {error}");
                return;
            }
        };
        if let Some(sink) = sink {
            if let Err(error) = sink.emit_changed(&payload) {
                log::warn!("[ExternalConfigMonitor] {error}");
            }
        }
    }
}

/// 由 AppState 持有的七模块外部配置检测服务。
pub struct ExternalConfigMonitor {
    db: Arc<Database>,
    proxy_service: ProxyService,
    state_store: Arc<ExternalConfigStateStore>,
    event_sink: Arc<StdRwLock<Option<Arc<dyn ExternalConfigEventSink>>>>,
    capture_source: Arc<dyn MonitorCaptureSource>,
    options: MonitorOptions,
    worker: Mutex<MonitorWorker>,
}

impl ExternalConfigMonitor {
    pub fn new(db: Arc<Database>, proxy_service: ProxyService) -> Self {
        Self::with_dependencies(
            db,
            proxy_service,
            Arc::new(SystemMonitorCaptureSource),
            MonitorOptions::default(),
        )
    }

    fn with_dependencies(
        db: Arc<Database>,
        proxy_service: ProxyService,
        capture_source: Arc<dyn MonitorCaptureSource>,
        options: MonitorOptions,
    ) -> Self {
        Self {
            db,
            proxy_service,
            state_store: Arc::new(ExternalConfigStateStore::new()),
            event_sink: Arc::new(StdRwLock::new(None)),
            capture_source,
            options,
            worker: Mutex::new(MonitorWorker::default()),
        }
    }

    /// 设置生产事件出口。setup 在启动 worker 前调用一次。
    pub fn set_app_handle(&self, app_handle: tauri::AppHandle) {
        match self.event_sink.write() {
            Ok(mut sink) => *sink = Some(Arc::new(AppHandleEventSink(app_handle))),
            Err(error) => log::error!("[ExternalConfigMonitor] 设置 AppHandle 失败: {error}"),
        }
    }

    /// 幂等启动：一个 service 实例一生只启动一个 worker，停止后不会隐式重启。
    pub async fn start(&self) -> Result<bool, String> {
        let mut worker = self.worker.lock().await;
        if worker.started_once {
            return Ok(false);
        }

        let (stop_tx, stop_rx) = oneshot::channel();
        let runtime = MonitorRuntime {
            db: self.db.clone(),
            proxy_service: self.proxy_service.clone(),
            state_store: self.state_store.clone(),
            event_sink: self.event_sink.clone(),
            capture_source: self.capture_source.clone(),
            options: self.options,
        };
        let join = tokio::spawn(runtime.run(stop_rx));
        worker.started_once = true;
        worker.stop_tx = Some(stop_tx);
        worker.join = Some(join);
        Ok(true)
    }

    /// 发送停止信号并等待 worker 完整退出；重复停止是无操作。
    pub async fn stop(&self) -> Result<bool, String> {
        let (stop_tx, join) = {
            let mut worker = self.worker.lock().await;
            let stop_tx = worker.stop_tx.take();
            let join = worker.join.take();
            (stop_tx, join)
        };

        if stop_tx.is_none() && join.is_none() {
            return Ok(false);
        }
        if let Some(stop_tx) = stop_tx {
            let _ = stop_tx.send(());
        }
        if let Some(join) = join {
            join.await
                .map_err(|error| format!("外部配置 monitor worker 异常终止: {error}"))?;
        }
        Ok(true)
    }

    /// 查询七模块实时只读状态；不创建任何新的持久化 SSOT。
    pub async fn get_status(&self) -> Result<Vec<ExternalConfigModuleStatus>, String> {
        let mut statuses = Vec::with_capacity(7);
        for app_type in AppType::all() {
            let config = self
                .db
                .get_proxy_config_for_app(app_type.as_str())
                .await
                .map_err(|error| format!("读取 {} 接管状态失败: {error}", app_type.as_str()))?;
            let external = self.state_store.module_state(&app_type).await;
            statuses.push(ExternalConfigModuleStatus {
                app_type: app_type.as_str().to_string(),
                generation: external.generation,
                conflict: external.conflict.is_some(),
                takeover_enabled: config.takeover_enabled,
                route_mode: config.route_mode,
            });
        }
        Ok(statuses)
    }

    /// Batch 3 writer integration 的窄入口。调用方仍须持有同一 C2 per-app switch lock。
    pub(crate) async fn begin_managed_write(&self, app_type: &AppType) -> Result<u64, String> {
        self.state_store.begin_managed_write(app_type).await
    }

    /// Batch 3 writer integration 的窄入口，以写后完整 target capture 提交 expected。
    pub(crate) async fn finish_managed_write(
        &self,
        app_type: &AppType,
        write_generation: u64,
        targets: Vec<ManagedTarget>,
    ) -> Result<ManagedExpected, String> {
        self.state_store
            .finish_managed_write(app_type, write_generation, targets)
            .await
    }

    #[cfg(test)]
    fn set_test_event_sink(&self, sink: Arc<dyn ExternalConfigEventSink>) {
        *self.event_sink.write().expect("event sink lock") = Some(sink);
    }

    #[cfg(test)]
    async fn worker_status(&self) -> (bool, bool) {
        let worker = self.worker.lock().await;
        (worker.started_once, worker.join.is_some())
    }
}

/// 只调用 C2 adapter 的内存 capture；不会调用 `capture_snapshot_once`，也不会访问 DAO。
/// 返回目标按稳定 id 排序，供 worker 在已有 per-app switch lock 内进行双次稳定采集。
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

fn monitored_target_paths(app_type: &AppType) -> Result<Vec<PathBuf>, String> {
    let mut paths = match app_type {
        AppType::Claude => vec![crate::config::get_claude_settings_path()],
        AppType::ClaudeDesktop => crate::claude_desktop_config::snapshot_target_paths()
            .map_err(|error| format!("解析 Claude Desktop 监控目标失败: {error}"))?
            .into_iter()
            .map(|(_, path)| path)
            .collect(),
        AppType::Codex => vec![
            crate::codex_config::get_codex_auth_path(),
            crate::codex_config::get_codex_config_path(),
            crate::codex_config::get_codex_model_catalog_path(),
        ],
        AppType::Gemini => vec![crate::gemini_config::get_gemini_env_path()],
        AppType::OpenCode => vec![crate::opencode_config::get_opencode_config_path()],
        AppType::OpenClaw => vec![crate::openclaw_config::get_openclaw_config_path()],
        AppType::Hermes => vec![crate::hermes_config::get_hermes_config_path()],
    };
    paths.sort();
    Ok(paths)
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
    use crate::provider::Provider;
    use serde_json::json;
    use serial_test::serial;
    use std::collections::{HashMap, VecDeque};
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex as StdMutex;
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

    #[derive(Default)]
    struct CollectingEventSink {
        payloads: StdMutex<Vec<ExternalConfigChangedPayload>>,
    }

    impl ExternalConfigEventSink for CollectingEventSink {
        fn emit_changed(&self, payload: &ExternalConfigChangedPayload) -> Result<(), String> {
            self.payloads.lock().unwrap().push(payload.clone());
            Ok(())
        }
    }

    impl CollectingEventSink {
        fn payloads(&self) -> Vec<ExternalConfigChangedPayload> {
            self.payloads.lock().unwrap().clone()
        }

        fn clear(&self) {
            self.payloads.lock().unwrap().clear();
        }
    }

    struct FakeModuleCapture {
        revision: u64,
        current: ManagedExpected,
        queued: VecDeque<Result<ManagedExpected, String>>,
        quick_error: Option<String>,
    }

    struct FakeCaptureSource {
        modules: StdMutex<HashMap<String, FakeModuleCapture>>,
    }

    impl FakeCaptureSource {
        fn new() -> Self {
            let modules = AppType::all()
                .map(|app_type| {
                    (
                        app_type.as_str().to_string(),
                        FakeModuleCapture {
                            revision: 1,
                            current: managed(&[("settings", Some(b"baseline"))]),
                            queued: VecDeque::new(),
                            quick_error: None,
                        },
                    )
                })
                .collect();
            Self {
                modules: StdMutex::new(modules),
            }
        }

        fn set_current(&self, app_type: &AppType, capture: ManagedExpected) {
            let mut modules = self.modules.lock().unwrap();
            let module = modules.get_mut(app_type.as_str()).unwrap();
            module.revision += 1;
            module.current = capture;
            module.queued.clear();
            module.quick_error = None;
        }

        fn queue_captures(
            &self,
            app_type: &AppType,
            captures: Vec<Result<ManagedExpected, String>>,
            fallback: ManagedExpected,
        ) {
            let mut modules = self.modules.lock().unwrap();
            let module = modules.get_mut(app_type.as_str()).unwrap();
            module.revision += 1;
            module.current = fallback;
            module.queued = captures.into();
            module.quick_error = None;
        }
    }

    impl MonitorCaptureSource for FakeCaptureSource {
        fn quick_signature(&self, app_type: &AppType) -> Result<QuickMetadataSignature, String> {
            let modules = self.modules.lock().unwrap();
            let module = modules.get(app_type.as_str()).unwrap();
            if let Some(error) = &module.quick_error {
                return Err(error.clone());
            }
            Ok(QuickMetadataSignature {
                targets: vec![QuickTargetMetadata {
                    path: PathBuf::from(app_type.as_str()),
                    existed: true,
                    len: module.revision,
                    modified_ns: Some(module.revision as u128),
                }],
            })
        }

        fn capture(&self, app_type: &AppType) -> Result<ManagedExpected, String> {
            let mut modules = self.modules.lock().unwrap();
            let module = modules.get_mut(app_type.as_str()).unwrap();
            module
                .queued
                .pop_front()
                .unwrap_or_else(|| Ok(module.current.clone()))
        }
    }

    fn test_options() -> MonitorOptions {
        MonitorOptions {
            poll_interval: Duration::from_millis(5),
            debounce_interval: Duration::from_millis(5),
            full_scan_interval: Duration::from_millis(10),
        }
    }

    fn fake_monitor(
        db: Arc<Database>,
        source: Arc<FakeCaptureSource>,
    ) -> Arc<ExternalConfigMonitor> {
        let proxy_service = ProxyService::new(db.clone());
        Arc::new(ExternalConfigMonitor::with_dependencies(
            db,
            proxy_service,
            source,
            test_options(),
        ))
    }

    fn bytes_target(id: &str, bytes: &[u8]) -> ManagedTarget {
        ManagedTarget::file_bytes(id, Some(bytes))
    }

    fn managed(targets: &[(&str, Option<&[u8]>)]) -> ManagedExpected {
        ManagedExpected::new(
            0,
            targets
                .iter()
                .map(|(id, bytes)| ManagedTarget::file_bytes(*id, *bytes))
                .collect(),
        )
        .unwrap()
    }

    async fn wait_for_events(sink: &CollectingEventSink, count: usize) {
        for _ in 0..200 {
            if sink.payloads().len() >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "等待事件超时：期望至少 {count}，实际 {}",
            sink.payloads().len()
        );
    }

    async fn wait_for_baselines(monitor: &ExternalConfigMonitor) {
        for _ in 0..200 {
            let mut ready = true;
            for app_type in AppType::all() {
                if monitor
                    .state_store
                    .module_state(&app_type)
                    .await
                    .last_observed
                    .is_none()
                {
                    ready = false;
                    break;
                }
            }
            if ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("等待七模块 baseline 超时");
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
    fn opencode_capture_and_metadata_only_use_opencode_json() {
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
        let quick = SystemMonitorCaptureSource
            .quick_signature(&AppType::OpenCode)
            .unwrap();

        assert_eq!(capture.targets.len(), 1);
        assert_eq!(capture.targets[0].id, "opencode.json");
        assert_eq!(capture.targets[0].bytes, config_bytes);
        assert_eq!(quick.targets.len(), 1);
        assert_eq!(quick.targets[0].path, config_path);
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

    #[tokio::test]
    async fn worker_start_is_idempotent_and_stop_awaits_join() {
        let db = Arc::new(Database::memory().unwrap());
        let monitor = fake_monitor(db, Arc::new(FakeCaptureSource::new()));

        assert!(monitor.start().await.unwrap());
        assert!(!monitor.start().await.unwrap());
        assert_eq!(monitor.worker_status().await, (true, true));
        assert!(monitor.stop().await.unwrap());
        assert_eq!(monitor.worker_status().await, (true, false));
        assert!(!monitor.stop().await.unwrap());
        assert!(
            !monitor.start().await.unwrap(),
            "停止后不得隐式创建第二个 worker"
        );
    }

    #[tokio::test]
    #[serial]
    async fn seven_unmanaged_modules_emit_once_without_mutating_live_or_providers() {
        let _home = TempHome::new();
        crate::settings::reload_settings().unwrap();
        let db = Arc::new(Database::memory().unwrap());
        for app_type in AppType::all() {
            let provider_id = format!("{}-provider", app_type.as_str());
            let provider = Provider::with_id(
                provider_id.clone(),
                format!("{} Provider", app_type.as_str()),
                json!({"marker": app_type.as_str()}),
                None,
            );
            db.save_provider(app_type.as_str(), &provider).unwrap();
            db.set_current_provider(app_type.as_str(), &provider_id)
                .unwrap();
        }
        let providers_before = AppType::all()
            .map(|app_type| {
                (
                    app_type.as_str().to_string(),
                    serde_json::to_value(db.get_all_providers(app_type.as_str()).unwrap()).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let current_before = AppType::all()
            .map(|app_type| {
                (
                    app_type.as_str().to_string(),
                    db.get_current_provider(app_type.as_str()).unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut config_values_before = HashMap::new();
        for app_type in AppType::all() {
            let config = db
                .get_proxy_config_for_app(app_type.as_str())
                .await
                .unwrap();
            config_values_before.insert(
                app_type.as_str().to_string(),
                (config.takeover_enabled, config.route_mode),
            );
        }

        let proxy_service = ProxyService::new(db.clone());
        let monitor = Arc::new(ExternalConfigMonitor::with_dependencies(
            db.clone(),
            proxy_service,
            Arc::new(SystemMonitorCaptureSource),
            test_options(),
        ));
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        sink.clear();

        let mut changed_targets = Vec::new();
        for (index, app_type) in AppType::all().enumerate() {
            let path = monitored_target_paths(&app_type).unwrap().remove(0);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let bytes = format!("external-{}-{index}", app_type.as_str()).into_bytes();
            fs::write(&path, &bytes).unwrap();
            changed_targets.push((app_type, path, bytes));
        }

        wait_for_events(&sink, 7).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let payloads = sink.payloads();
        assert_eq!(payloads.len(), 7);
        for app_type in AppType::all() {
            let payload = payloads
                .iter()
                .find(|payload| payload.app_type == app_type.as_str())
                .unwrap();
            assert!(!payload.conflict);
            assert!(!payload.takeover_enabled);
        }
        for (_, path, bytes) in changed_targets {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        for app_type in AppType::all() {
            assert_eq!(
                serde_json::to_value(db.get_all_providers(app_type.as_str()).unwrap()).unwrap(),
                providers_before[app_type.as_str()]
            );
            assert_eq!(
                db.get_current_provider(app_type.as_str()).unwrap(),
                current_before[app_type.as_str()]
            );
            let config = db
                .get_proxy_config_for_app(app_type.as_str())
                .await
                .unwrap();
            assert_eq!(
                (config.takeover_enabled, config.route_mode),
                config_values_before[app_type.as_str()]
            );
        }
        monitor.stop().await.unwrap();
    }

    #[tokio::test]
    async fn takeover_mismatch_preserves_first_expected_and_restore_snapshot() {
        let db = Arc::new(Database::memory().unwrap());
        let mut config = db.get_proxy_config_for_app("hermes").await.unwrap();
        config.takeover_enabled = true;
        config.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(config).await.unwrap();
        db.save_live_backup("hermes", "immutable-restore-snapshot")
            .await
            .unwrap();

        let source = Arc::new(FakeCaptureSource::new());
        let monitor = fake_monitor(db.clone(), source.clone());
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        let baseline = monitor
            .state_store
            .module_state(&AppType::Hermes)
            .await
            .expected
            .unwrap();
        sink.clear();

        source.set_current(
            &AppType::Hermes,
            managed(&[("config.yaml", Some(b"external-1"))]),
        );
        wait_for_events(&sink, 1).await;
        source.set_current(
            &AppType::Hermes,
            managed(&[("config.yaml", Some(b"external-2"))]),
        );
        wait_for_events(&sink, 2).await;

        source.set_current(&AppType::Hermes, baseline.clone());
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(
            sink.payloads().len(),
            2,
            "外部内容自行回到 expected 时后台仍不得自动解决冲突"
        );

        let state = monitor.state_store.module_state(&AppType::Hermes).await;
        let conflict = state.conflict.unwrap();
        assert_eq!(conflict.expected.fingerprint, baseline.fingerprint);
        assert_eq!(state.expected.unwrap().fingerprint, baseline.fingerprint);
        assert_eq!(conflict.observed.targets[0].bytes, b"external-2");
        assert_eq!(
            db.get_live_backup("hermes")
                .await
                .unwrap()
                .unwrap()
                .original_config,
            "immutable-restore-snapshot"
        );
        assert!(sink.payloads().iter().all(|payload| payload.conflict));

        let statuses = monitor.get_status().await.unwrap();
        assert_eq!(statuses.len(), 7);
        let hermes = statuses
            .iter()
            .find(|status| status.app_type == "hermes")
            .unwrap();
        assert!(hermes.conflict);
        assert!(hermes.takeover_enabled);
        assert_eq!(hermes.route_mode, RouteMode::Proxy);
        let wire = serde_json::to_value(hermes).unwrap();
        assert_eq!(wire["appType"], "hermes");
        assert_eq!(wire["takeoverEnabled"], true);
        assert_eq!(wire["routeMode"], "proxy");
        monitor.stop().await.unwrap();
    }

    #[tokio::test]
    async fn ownership_turn_on_initializes_expected_without_file_metadata_change() {
        let db = Arc::new(Database::memory().unwrap());
        let source = Arc::new(FakeCaptureSource::new());
        let monitor = fake_monitor(db.clone(), source);
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        sink.clear();

        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.takeover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        for _ in 0..100 {
            if monitor
                .state_store
                .module_state(&AppType::Claude)
                .await
                .expected
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(monitor
            .state_store
            .module_state(&AppType::Claude)
            .await
            .expected
            .is_some());
        assert!(sink.payloads().is_empty());
        monitor.stop().await.unwrap();
    }

    #[tokio::test]
    async fn expected_equal_and_inflight_managed_write_are_suppressed() {
        let db = Arc::new(Database::memory().unwrap());
        let mut config = db.get_proxy_config_for_app("claude").await.unwrap();
        config.takeover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();

        let source = Arc::new(FakeCaptureSource::new());
        let monitor = fake_monitor(db, source.clone());
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        sink.clear();

        source.set_current(
            &AppType::Claude,
            managed(&[("settings", Some(b"baseline"))]),
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(sink.payloads().is_empty(), "expected-equal 不应发事件");

        let write_generation = monitor.begin_managed_write(&AppType::Claude).await.unwrap();
        let rewritten = managed(&[("settings", Some(b"ags-rewritten"))]);
        source.set_current(&AppType::Claude, rewritten.clone());
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(sink.payloads().is_empty(), "in-flight 写入不应发事件");

        monitor
            .finish_managed_write(
                &AppType::Claude,
                write_generation,
                rewritten.targets.clone(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(sink.payloads().is_empty(), "写后 expected 相等不应回环");
        assert!(monitor
            .state_store
            .module_state(&AppType::Claude)
            .await
            .conflict
            .is_none());
        monitor.stop().await.unwrap();
    }

    #[tokio::test]
    async fn read_error_and_transient_missing_target_retry_without_event() {
        let db = Arc::new(Database::memory().unwrap());
        let source = Arc::new(FakeCaptureSource::new());
        let monitor = fake_monitor(db, source.clone());
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        sink.clear();

        let baseline = managed(&[("settings", Some(b"baseline"))]);
        source.queue_captures(
            &AppType::Claude,
            vec![
                Err("injected read error".to_string()),
                Ok(managed(&[("settings", None)])),
                Ok(baseline.clone()),
                Ok(baseline.clone()),
            ],
            baseline,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(sink.payloads().is_empty());
        assert!(monitor
            .state_store
            .module_state(&AppType::Claude)
            .await
            .conflict
            .is_none());
        monitor.stop().await.unwrap();
    }

    #[tokio::test]
    async fn multi_target_intermediate_capture_debounces_to_one_stable_event() {
        let db = Arc::new(Database::memory().unwrap());
        let source = Arc::new(FakeCaptureSource::new());
        source.set_current(
            &AppType::Codex,
            managed(&[
                ("auth", Some(b"old-auth")),
                ("config", Some(b"old-config")),
                ("model_catalog", Some(b"old-catalog")),
            ]),
        );
        let monitor = fake_monitor(db, source.clone());
        let sink = Arc::new(CollectingEventSink::default());
        monitor.set_test_event_sink(sink.clone());
        monitor.start().await.unwrap();
        wait_for_baselines(&monitor).await;
        sink.clear();

        let partial = managed(&[
            ("auth", Some(b"new-auth")),
            ("config", Some(b"old-config")),
            ("model_catalog", Some(b"old-catalog")),
        ]);
        let stable = managed(&[
            ("auth", Some(b"new-auth")),
            ("config", Some(b"new-config")),
            ("model_catalog", Some(b"new-catalog")),
        ]);
        source.queue_captures(
            &AppType::Codex,
            vec![Ok(partial), Ok(stable.clone()), Ok(stable.clone())],
            stable.clone(),
        );

        wait_for_events(&sink, 1).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(sink.payloads().len(), 1);
        let state = monitor.state_store.module_state(&AppType::Codex).await;
        assert_eq!(state.last_observed.unwrap().fingerprint, stable.fingerprint);
        monitor.stop().await.unwrap();
    }
}
