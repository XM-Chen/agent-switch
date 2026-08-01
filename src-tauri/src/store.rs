use crate::database::Database;
use crate::services::ProxyService;
use std::sync::Arc;

/// 独立网关的全局应用状态，只持有 Agent Switch 自身数据库与网关服务。
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub proxy_service: ProxyService,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            proxy_service: ProxyService::new(db.clone()),
            db,
        }
    }
}
