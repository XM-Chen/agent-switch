use std::str::FromStr;
use tauri::{Emitter, State};

use crate::app_config::AppType;
use crate::services::subscription::SubscriptionQuota;
use crate::store::AppState;

/// 查询官方订阅额度
///
/// 读取 CLI 工具已有的 OAuth 凭据并调用官方 API 获取使用额度。
/// 业务结果写入 `UsageCache` 并通知托盘；transport 层 Err 会失效托盘缓存，
/// 避免旧额度永久滞留。Err 原样向前端返回，由 React Query 的 keep-last-good
/// 时间窗负责短期展示。
#[tauri::command]
pub async fn get_subscription_quota(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    tool: String,
) -> Result<SubscriptionQuota, String> {
    let inner = crate::services::subscription::get_subscription_quota(&tool).await;
    if let Ok(snapshot) = &inner {
        if let Ok(app_type) = AppType::from_str(&tool) {
            let payload = serde_json::json!({
                "kind": "subscription",
                "appType": app_type.as_str(),
                "data": snapshot,
            });
            if let Err(e) = app.emit("usage-cache-updated", payload) {
                log::error!("emit usage-cache-updated (subscription) 失败: {e}");
            }
            state
                .usage_cache
                .put_subscription(app_type, snapshot.clone());
            crate::tray::schedule_tray_refresh(&app);
        }
    } else if let Ok(app_type) = AppType::from_str(&tool) {
        state.usage_cache.invalidate_subscription(&app_type);
        crate::tray::schedule_tray_refresh(&app);
    }
    inner
}
