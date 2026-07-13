//! 上游模型缓存与刷新调度（CC 聚合功能地基，C1）
//!
//! 职责：
//! - `refresh_one` / `refresh_all_queue_members`：拉取上游 `/v1/models` 并按覆盖
//!   规则落库（复用 `services/model_fetch`）。
//! - 防抖增量刷新：新增/移入队列且此前未缓存的上游触发，60s 防抖合并（照抄
//!   `services/webdav_auto_sync` 的 mpsc + debounce 范式）。
//! - 每日全量刷新：每天上海时间 04:00 + 0~30min 抖动 对全部队列成员拉取；应用
//!   启动时若错过最近应发生的 04:00 则立即补跑（错过补跑，父任务 D8）。
//!
//! 本模块**不**做聚合派生（C2）、不改路由（C3）、不涉及前端（C4）。托管账号类
//! 上游（Codex OAuth / Copilot / Anthropic 原生，取不到 `/v1/models`）拉取失败
//! 时完全跳过，不写缓存、不阻断其他上游与应用启动。

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, NaiveDate, TimeZone, Utc};
use serde::Serialize;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use crate::app_config::AppType;
use crate::database::Database;
use crate::error::AppError;
use crate::services::model_fetch;
use std::str::FromStr;

/// 每日全量刷新 last-run 的 settings 键（RFC3339 字符串）。
const LAST_FULL_REFRESH_KEY: &str = "cc_model_cache_last_full_refresh";

/// 防抖窗口：最后一次触发后静默满该时长才执行（父任务 D8：约 1 分钟）。
const DEBOUNCE_SECS: u64 = 60;

/// 抖动上限：每日 04:00 之后追加 0~JITTER_MAX_SECS 的随机延迟，避免被识别为机器行为。
const JITTER_MAX_SECS: u64 = 30 * 60;

/// 无法计算下一次 04:00 时的退避重试间隔。
const SCHEDULER_RETRY_SECS: u64 = 60 * 60;

/// 单个上游刷新结果。
#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    /// 成功覆盖 fetched 行，`count` 为本次落库的 fetched 模型数。
    Refreshed { count: usize },
    /// 跳过（无凭据的托管账号类 / provider 不存在 / fetch 失败），`reason` 供日志。
    Skipped { reason: String },
}

/// 队列全量刷新汇总。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSummary {
    /// 成功刷新的上游数。
    pub refreshed: usize,
    /// 跳过的上游数（含托管账号类与 fetch 失败）。
    pub skipped: usize,
    /// 本次总计落库的 fetched 模型数。
    pub total_models: usize,
}

/// 单个上游的缓存状态（供 C4 展示）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCacheStatus {
    pub provider_id: String,
    pub model_count: usize,
    /// 该上游任意缓存行中最新的 `fetched_at`（毫秒 epoch）。
    pub latest_fetched_at: i64,
}

/// 模型缓存整体状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCacheStatus {
    /// 每日全量刷新上次执行时刻（RFC3339），从未执行为 `None`。
    pub last_full_refresh: Option<String>,
    pub providers: Vec<ProviderCacheStatus>,
}

// ── 拉取编排 ────────────────────────────────────────────────────────────────

/// 对单个上游执行 fetch 并按覆盖规则落库。
///
/// 托管账号类（无凭据）或 fetch 失败 → `Ok(Skipped)`，不写缓存、不传播错误，
/// 保证不阻断其他上游与应用启动（R7 / R9）。
pub async fn refresh_one(
    db: &Database,
    app_type: &str,
    provider_id: &str,
) -> Result<RefreshOutcome, AppError> {
    let Some(provider) = db.get_provider_by_id(provider_id, app_type)? else {
        return Ok(RefreshOutcome::Skipped {
            reason: "provider not found".to_string(),
        });
    };

    let app_enum = AppType::from_str(app_type)
        .map_err(|_| AppError::Config(format!("无效的应用类型: {app_type}")))?;

    let (base_url, api_key) = provider.resolve_usage_credentials(&app_enum);
    // 托管账号类（Codex OAuth / Copilot / Anthropic 原生）拿不到可用于 /v1/models
    // 的 (base_url, api_key)，完全跳过（R7 / D4）。
    if base_url.is_empty() || api_key.is_empty() {
        return Ok(RefreshOutcome::Skipped {
            reason: "no usable credentials for /v1/models".to_string(),
        });
    }

    let is_full_url = provider
        .meta
        .as_ref()
        .and_then(|m| m.is_full_url)
        .unwrap_or(false);
    let user_agent = provider
        .meta
        .as_ref()
        .and_then(|m| m.custom_user_agent_header().ok().flatten());

    // models_url 不持久化（本任务简化）：走默认候选 URL 兜底（含兼容子路径剥离）。
    match model_fetch::fetch_models(&base_url, &api_key, is_full_url, None, user_agent).await {
        Ok(models) => {
            let fetched_at = Utc::now().timestamp_millis();
            db.replace_fetched_models(app_type, provider_id, &models, fetched_at)?;
            Ok(RefreshOutcome::Refreshed {
                count: models.len(),
            })
        }
        Err(reason) => {
            // fetch 返回 String 错误：统一 warn，不传播（避免打断队列其他成员）。
            log::warn!("[ModelCache] fetch models failed for {provider_id} ({app_type}): {reason}");
            Ok(RefreshOutcome::Skipped { reason })
        }
    }
}

/// 对故障转移队列全部成员逐个 `refresh_one`（失败只 warn 不中断，R5 / R9）。
pub async fn refresh_all_queue_members(
    db: &Database,
    app_type: &str,
) -> Result<RefreshSummary, AppError> {
    let queue = db.get_failover_queue(app_type)?;
    let mut summary = RefreshSummary::default();

    for item in queue {
        match refresh_one(db, app_type, &item.provider_id).await {
            Ok(RefreshOutcome::Refreshed { count }) => {
                summary.refreshed += 1;
                summary.total_models += count;
            }
            Ok(RefreshOutcome::Skipped { reason }) => {
                summary.skipped += 1;
                log::debug!(
                    "[ModelCache] skipped {} ({app_type}): {reason}",
                    item.provider_id
                );
            }
            Err(e) => {
                summary.skipped += 1;
                log::warn!(
                    "[ModelCache] refresh_one errored for {} ({app_type}): {e}",
                    item.provider_id
                );
            }
        }
    }

    Ok(summary)
}

/// 组装模型缓存状态（last-run + 各上游最近刷新时间），供 C4 展示。
pub fn get_status(db: &Database, app_type: &str) -> Result<ModelCacheStatus, AppError> {
    let last_full_refresh = db.get_setting(LAST_FULL_REFRESH_KEY)?;
    let rows = db.list_provider_models(app_type, None)?;

    let mut providers: Vec<ProviderCacheStatus> = Vec::new();
    for row in rows {
        if let Some(entry) = providers
            .iter_mut()
            .find(|p| p.provider_id == row.provider_id)
        {
            entry.model_count += 1;
            entry.latest_fetched_at = entry.latest_fetched_at.max(row.fetched_at);
        } else {
            providers.push(ProviderCacheStatus {
                provider_id: row.provider_id,
                model_count: 1,
                latest_fetched_at: row.fetched_at,
            });
        }
    }

    Ok(ModelCacheStatus {
        last_full_refresh,
        providers,
    })
}

// ── 防抖增量刷新 ─────────────────────────────────────────────────────────────

/// (app_type, provider_id)
type RefreshKey = (String, String);

static REFRESH_TX: OnceLock<Sender<RefreshKey>> = OnceLock::new();

/// 防抖触发入口：把上游加入待刷新集合并重置防抖计时。
///
/// **触发约定（调用方负责，R14）**：仅在「新增/移入队列且该上游此前无任何缓存行」
/// 时调用；调序 / 删除 / 移出队列**不**调用。worker 未启动（如测试）时静默丢弃。
pub fn notify_provider_needs_refresh(app_type: &str, provider_id: &str) {
    let Some(tx) = REFRESH_TX.get() else {
        return;
    };
    // 缓冲区满属于极端突发，丢弃即可：每日全量刷新会兜底校准。
    let _ = tx.try_send((app_type.to_string(), provider_id.to_string()));
}

fn start_debounce_worker(db: Arc<Database>) {
    if REFRESH_TX.get().is_some() {
        return;
    }
    // 缓冲区略大，避免密集移入时不同上游信号被丢弃（webdav 只需 dirty 信号故用 1）。
    let (tx, rx) = channel::<RefreshKey>(64);
    if REFRESH_TX.set(tx).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        run_debounce_loop(db, rx).await;
    });
}

async fn run_debounce_loop(db: Arc<Database>, mut rx: Receiver<RefreshKey>) {
    while let Some(first) = rx.recv().await {
        let mut pending: HashSet<RefreshKey> = HashSet::new();
        pending.insert(first);

        // 防抖：最后一次信号后静默满 DEBOUNCE_SECS 才执行；期间新信号合并并重置计时。
        // 通道关闭（None）或窗口静默（超时 Err）时退出内层循环，处理已积攒的集合。
        while let Ok(Some(key)) =
            tokio::time::timeout(Duration::from_secs(DEBOUNCE_SECS), rx.recv()).await
        {
            pending.insert(key);
        }

        for (app_type, provider_id) in pending.drain() {
            match refresh_one(&db, &app_type, &provider_id).await {
                Ok(outcome) => {
                    log::debug!(
                        "[ModelCache] debounce refresh {provider_id} ({app_type}): {outcome:?}"
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[ModelCache] debounce refresh {provider_id} ({app_type}) failed: {e}"
                    );
                }
            }
        }
    }
}

// ── 每日全量刷新调度 ─────────────────────────────────────────────────────────

/// 启动钩子：挂防抖 worker + 启动错过补跑 + 每日 04:00(上海)+抖动 循环。
///
/// 照 `lib.rs` 的「启动先跑 + 周期」范式，在 `.setup()` 内以 `state.db.clone()` 调用。
pub fn start_schedulers(db: Arc<Database>) {
    start_debounce_worker(db.clone());
    tauri::async_runtime::spawn(async move {
        run_daily_scheduler(db).await;
    });
}

async fn run_daily_scheduler(db: Arc<Database>) {
    // 1. 启动补跑：错过最近应发生的 04:00（或从未跑过）→ 立即全量一次（抖动不适用补跑）。
    let last_run = read_last_full_refresh(&db);
    if should_catch_up(last_run, Utc::now()) {
        log::info!("[ModelCache] startup catch-up: missed daily 04:00 full refresh, running now");
        run_full_refresh(&db).await;
    }

    // 2. 循环：睡到下一个 04:00(上海) + 随机抖动，执行全量刷新。
    loop {
        let now = Utc::now();
        let Some(next) = next_shanghai_0400(now) else {
            log::warn!("[ModelCache] cannot compute next 04:00, retrying in 1h");
            tokio::time::sleep(Duration::from_secs(SCHEDULER_RETRY_SECS)).await;
            continue;
        };

        let base_wait = (next - now).to_std().unwrap_or(Duration::from_secs(0));
        let wait = base_wait + Duration::from_secs(startup_jitter_secs());
        log::debug!(
            "[ModelCache] next daily full refresh in {}s",
            wait.as_secs()
        );
        tokio::time::sleep(wait).await;

        run_full_refresh(&db).await;
    }
}

async fn run_full_refresh(db: &Database) {
    match refresh_all_queue_members(db, AppType::Claude.as_str()).await {
        Ok(summary) => {
            log::info!("[ModelCache] daily full refresh done: {summary:?}");
            write_last_full_refresh(db, Utc::now());
        }
        Err(e) => {
            log::warn!("[ModelCache] daily full refresh failed: {e}");
        }
    }
}

fn read_last_full_refresh(db: &Database) -> Option<DateTime<Utc>> {
    let raw = db.get_setting(LAST_FULL_REFRESH_KEY).ok().flatten()?;
    DateTime::parse_from_rfc3339(&raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn write_last_full_refresh(db: &Database, now: DateTime<Utc>) {
    if let Err(e) = db.set_setting(LAST_FULL_REFRESH_KEY, &now.to_rfc3339()) {
        log::warn!("[ModelCache] failed to persist last full refresh time: {e}");
    }
}

/// 抖动秒数（0~JITTER_MAX_SECS）。用系统纳秒取模，避免引入 rand 依赖；此处随机性
/// 只用于打散固定 04:00，非安全用途，弱随机足够。
fn startup_jitter_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()) % (JITTER_MAX_SECS + 1))
        .unwrap_or(0)
}

/// 上海时区固定偏移的秒数（+08:00）。
const SHANGHAI_OFFSET_SECS: i32 = 8 * 3600;

/// 上海时区固定偏移 +08:00（不用 `Local`，避免机器时区非上海时算错）。
///
/// `8*3600` 秒在 `east_opt` 的 ±86_400 合法范围内，恒返回 `Some`；`None` 分支
/// （不可达）仅为消除 unwrap/expect。
fn shanghai_offset() -> Option<FixedOffset> {
    FixedOffset::east_opt(SHANGHAI_OFFSET_SECS)
}

fn shanghai_0400_on(offset: &FixedOffset, date: NaiveDate) -> Option<DateTime<Utc>> {
    let naive = date.and_hms_opt(4, 0, 0)?;
    offset
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 最近一次「应已发生」的上海 04:00（≤ now 的最近一个 04:00）。
fn most_recent_shanghai_0400(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let offset = shanghai_offset()?;
    let now_sh = now.with_timezone(&offset);
    let today = shanghai_0400_on(&offset, now_sh.date_naive())?;
    if today <= now {
        Some(today)
    } else {
        shanghai_0400_on(&offset, now_sh.date_naive() - ChronoDuration::days(1))
    }
}

/// 严格晚于 now 的下一个上海 04:00。
fn next_shanghai_0400(now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let offset = shanghai_offset()?;
    let now_sh = now.with_timezone(&offset);
    let today = shanghai_0400_on(&offset, now_sh.date_naive())?;
    if today > now {
        Some(today)
    } else {
        shanghai_0400_on(&offset, now_sh.date_naive() + ChronoDuration::days(1))
    }
}

/// 启动补跑判定：从未跑过，或上次刷新早于最近应发生的 04:00（R17）。
fn should_catch_up(last_run: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    let Some(threshold) = most_recent_shanghai_0400(now) else {
        return false;
    };
    match last_run {
        None => true,
        Some(last) => last < threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        let offset = shanghai_offset().expect("shanghai offset valid");
        offset
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("valid shanghai time")
            .with_timezone(&Utc)
    }

    #[test]
    fn most_recent_0400_is_today_when_after_0400() {
        // 上海 2026-07-13 10:00 → 最近应发生的是当天 04:00。
        let now = sh(2026, 7, 13, 10, 0);
        let expected = sh(2026, 7, 13, 4, 0);
        assert_eq!(most_recent_shanghai_0400(now), Some(expected));
    }

    #[test]
    fn most_recent_0400_is_yesterday_when_before_0400() {
        // 上海 2026-07-13 02:00（04:00 未到）→ 最近应发生的是前一天 04:00。
        let now = sh(2026, 7, 13, 2, 0);
        let expected = sh(2026, 7, 12, 4, 0);
        assert_eq!(most_recent_shanghai_0400(now), Some(expected));
    }

    #[test]
    fn next_0400_is_tomorrow_when_after_0400() {
        let now = sh(2026, 7, 13, 10, 0);
        let expected = sh(2026, 7, 14, 4, 0);
        assert_eq!(next_shanghai_0400(now), Some(expected));
    }

    #[test]
    fn next_0400_is_today_when_before_0400() {
        let now = sh(2026, 7, 13, 2, 0);
        let expected = sh(2026, 7, 13, 4, 0);
        assert_eq!(next_shanghai_0400(now), Some(expected));
    }

    #[test]
    fn catch_up_when_never_run() {
        let now = sh(2026, 7, 13, 10, 0);
        assert!(should_catch_up(None, now));
    }

    #[test]
    fn catch_up_when_last_run_before_todays_0400() {
        // now 是当天 10:00，上次刷新是前一天 23:00（< 当天 04:00）→ 需补跑。
        let now = sh(2026, 7, 13, 10, 0);
        let last = sh(2026, 7, 12, 23, 0);
        assert!(should_catch_up(Some(last), now));
    }

    #[test]
    fn no_catch_up_when_last_run_after_todays_0400() {
        // now 当天 10:00，上次刷新当天 05:00（≥ 当天 04:00）→ 不补跑。
        let now = sh(2026, 7, 13, 10, 0);
        let last = sh(2026, 7, 13, 5, 0);
        assert!(!should_catch_up(Some(last), now));
    }

    #[test]
    fn no_catch_up_before_0400_when_last_run_after_yesterday_0400() {
        // now 当天 02:00（阈值=前一天 04:00），上次刷新前一天 05:00 → 不补跑。
        let now = sh(2026, 7, 13, 2, 0);
        let last = sh(2026, 7, 12, 5, 0);
        assert!(!should_catch_up(Some(last), now));
    }

    #[test]
    fn jitter_within_bounds() {
        assert!(startup_jitter_secs() <= JITTER_MAX_SECS);
    }
}
