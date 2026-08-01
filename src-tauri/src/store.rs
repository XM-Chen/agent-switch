use crate::database::Database;
use crate::services::{ProxyService, UsageCache};
use std::sync::Arc;

/// 独立网关的全局应用状态，只持有 Agent Switch 自身数据库与网关服务。
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
    pub usage_cache: Arc<UsageCache>,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            proxy_service: ProxyService::new(db.clone()),
            db,
            usage_cache: Arc::new(UsageCache::new()),
        }
    }
}
