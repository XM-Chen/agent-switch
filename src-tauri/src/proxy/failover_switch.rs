//! 故障转移切换通知模块。
//!
//! 模型优先路由下，故障转移由 `ProviderRouter` 的 route_target 熔断器与候选序列
//! 处理。本模块只在转发实际落到非首选上游时，向前端发射 `provider-switched`
//! 事件，便于 UI 反映当前使用的上游。它不再触碰任何客户端配置，也不维护
//! "当前供应商"接管态。

use crate::error::AppError;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

/// 故障转移切换通知器：对相同 `(app, provider)` 去重，避免并发请求重复发射事件。
#[derive(Clone)]
pub struct FailoverSwitchManager {
    pending_switches: Arc<RwLock<HashSet<String>>>,
}

impl FailoverSwitchManager {
    pub fn new(_db: Arc<crate::database::Database>) -> Self {
        Self {
            pending_switches: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// 发射故障转移切换通知。返回是否实际发射（未因去重跳过）。
    pub async fn try_switch(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        app_type: &str,
        provider_id: &str,
        provider_name: &str,
    ) -> Result<bool, AppError> {
        let switch_key = format!("{app_type}:{provider_id}");

        {
            let mut pending = self.pending_switches.write().await;
            if pending.contains(&switch_key) {
                log::debug!("[Failover] 切换通知已在进行中，跳过: {app_type} -> {provider_id}");
                return Ok(false);
            }
            pending.insert(switch_key.clone());
        }

        let emitted = self.emit_event(app_handle, app_type, provider_id, provider_name);

        {
            let mut pending = self.pending_switches.write().await;
            pending.remove(&switch_key);
        }
        Ok(emitted)
    }

    fn emit_event(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        app_type: &str,
        provider_id: &str,
        provider_name: &str,
    ) -> bool {
        let Some(app) = app_handle else {
            return false;
        };
        let event_data = serde_json::json!({
            "appType": app_type,
            "providerId": provider_id,
            "providerName": provider_name,
            "source": "failover"
        });
        if let Err(e) = app.emit("provider-switched", event_data) {
            log::error!("[Failover] 发射事件失败: {e}");
            return false;
        }
        // 失败转移成功后刷新托盘菜单，让用户看到当前上游。
        if let Some(app_state) = app.try_state::<crate::store::AppState>() {
            if let Ok(new_menu) = crate::tray::create_tray_menu(app, app_state.inner()) {
                if let Some(tray) = app.tray_by_id(crate::tray::TRAY_ID) {
                    if let Err(e) = tray.set_menu(Some(new_menu)) {
                        log::error!("[Failover] 更新托盘菜单失败: {e}");
                    }
                }
            }
        }
        true
    }
}
