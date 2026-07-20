use crate::database::Database;
use crate::services::{ExternalConfigMonitor, ProxyService, UsageCache};
use std::sync::Arc;

/// 全局应用状态。clone 只复制共享服务句柄；ProxyService、monitor、锁与 worker
/// 仍是同一运行时实例，供 blocking 后置同步安全复用。
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
    pub external_config_monitor: Arc<ExternalConfigMonitor>,
    pub usage_cache: Arc<UsageCache>,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        let proxy_service = ProxyService::new(db.clone());
        let external_config_monitor = Arc::new(ExternalConfigMonitor::new(
            db.clone(),
            proxy_service.clone(),
        ));
        proxy_service.set_external_config_monitor(Arc::downgrade(&external_config_monitor));

        Self {
            db,
            proxy_service,
            external_config_monitor,
            usage_cache: Arc::new(UsageCache::new()),
        }
    }
}
