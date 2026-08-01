//! 独立本地模型网关生命周期服务。
//!
//! 只负责网关 HTTP 服务的启动、停止、状态、配置与熔断器热更新。
//! 不接管、不快照、不探测、不恢复任何客户端配置文件--客户端配置属于
//! 不可触碰的外部边界，Agent Switch 只作为独立的本地模型网关运行。

use crate::database::Database;
use crate::proxy::server::ProxyServer;
use crate::proxy::types::*;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 独立网关服务。持有共享的 [`ProxyServer`] 句柄与 AppHandle。
///
/// `Clone` 廉价：内部状态全部 `Arc` 共享，所有副本指向同一个运行中的服务器。
#[derive(Clone)]
pub struct ProxyService {
    db: Arc<Database>,
    server: Arc<RwLock<Option<ProxyServer>>>,
    /// AppHandle，用于传递给 ProxyServer 以支持故障转移时的 UI 更新。
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
}

impl ProxyService {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            server: Arc::new(RwLock::new(None)),
            app_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// 注入 AppHandle，供运行中的 ProxyServer 发射事件与更新托盘菜单。
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut guard) = self.app_handle.try_write() {
            *guard = Some(handle);
        }
    }

    /// 启动独立本地网关。
    ///
    /// 只读取/写入 Agent Switch 自有数据库中的网关配置，不触达任何客户端配置。
    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        // 1. 启动时自动设置 proxy_enabled = true
        let mut global_config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| format!("获取全局代理配置失败: {e}"))?;

        if !global_config.proxy_enabled {
            global_config.proxy_enabled = true;
            self.db
                .update_global_proxy_config(global_config.clone())
                .await
                .map_err(|e| format!("更新代理总开关失败: {e}"))?;
        }

        // 2. 获取配置
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))?;

        // 3. 若已在运行：返回当前信息
        if let Some(server) = self.server.read().await.as_ref() {
            let status = server.get_status().await;
            return Ok(ProxyServerInfo {
                address: status.address,
                port: status.port,
                // 无法精确取回首次启动时间，返回当前时间用于 UI 展示即可
                started_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        // 4. 创建并启动服务器
        let app_handle = self.app_handle.read().await.clone();
        let server = ProxyServer::new(config.clone(), self.db.clone(), app_handle);
        let info = server
            .start()
            .await
            .map_err(|e| format!("启动代理服务器失败: {e}"))?;
        if let Err(e) = self
            .persist_ephemeral_listen_port_if_needed(&config, info.port)
            .await
        {
            let _ = server.stop().await;
            return Err(e);
        }

        // 5. 保存服务器实例
        *self.server.write().await = Some(server);

        log::info!("代理服务器已启动: {}:{}", info.address, info.port);
        Ok(info)
    }

    /// 监听端口为 0（系统分配）时，把实际端口持久化回数据库，便于下次启动复用。
    async fn persist_ephemeral_listen_port_if_needed(
        &self,
        config: &ProxyConfig,
        actual_port: u16,
    ) -> Result<(), String> {
        if config.listen_port != 0 {
            return Ok(());
        }

        let mut resolved_config = config.clone();
        resolved_config.listen_port = actual_port;
        self.db
            .update_proxy_config(resolved_config)
            .await
            .map_err(|e| format!("保存动态代理端口失败: {e}"))
    }

    /// 停止本进程内的网关。独立网关不检查或恢复任何客户端接管状态。
    pub async fn stop(&self) -> Result<(), String> {
        if let Some(server) = self.server.write().await.take() {
            server
                .stop()
                .await
                .map_err(|e| format!("停止代理服务器失败: {e}"))?;

            // 停止时设置 proxy_enabled = false
            let mut global_config = self
                .db
                .get_global_proxy_config()
                .await
                .map_err(|e| format!("获取全局代理配置失败: {e}"))?;

            if global_config.proxy_enabled {
                global_config.proxy_enabled = false;
                if let Err(e) = self.db.update_global_proxy_config(global_config).await {
                    log::warn!("更新代理总开关失败: {e}");
                }
            }

            log::info!("代理服务器已停止");
            Ok(())
        } else {
            Err("代理服务器未运行".to_string())
        }
    }

    pub async fn get_status(&self) -> Result<ProxyStatus, String> {
        if let Some(server) = self.server.read().await.as_ref() {
            Ok(server.get_status().await)
        } else {
            Ok(ProxyStatus {
                running: false,
                ..Default::default()
            })
        }
    }

    /// 获取代理配置。
    pub async fn get_config(&self) -> Result<ProxyConfig, String> {
        self.db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))
    }

    /// 更新代理配置；地址/端口变更时自动重启服务器，其余实时应用。
    pub async fn update_config(&self, config: &ProxyConfig) -> Result<(), String> {
        // 记录旧配置用于判定是否需要重启
        let previous = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))?;

        // 保存到数据库（保持 live_takeover_active 状态不变）
        let mut new_config = config.clone();
        new_config.live_takeover_active = previous.live_takeover_active;

        self.db
            .update_proxy_config(new_config.clone())
            .await
            .map_err(|e| format!("保存代理配置失败: {e}"))?;

        // 检查服务器当前状态
        let mut server_guard = self.server.write().await;
        if server_guard.is_none() {
            return Ok(());
        }

        // 判断是否需要重启（地址或端口变更）
        let require_restart = new_config.listen_address != previous.listen_address
            || new_config.listen_port != previous.listen_port;

        if require_restart {
            if let Some(server) = server_guard.take() {
                server
                    .stop()
                    .await
                    .map_err(|e| format!("重启前停止代理服务器失败: {e}"))?;
            }

            let app_handle = self.app_handle.read().await.clone();
            let new_server = ProxyServer::new(new_config.clone(), self.db.clone(), app_handle);
            let info = new_server
                .start()
                .await
                .map_err(|e| format!("重启代理服务器失败: {e}"))?;
            if let Err(e) = self
                .persist_ephemeral_listen_port_if_needed(&new_config, info.port)
                .await
            {
                let _ = new_server.stop().await;
                return Err(e);
            }

            *server_guard = Some(new_server);
            log::info!("代理配置已更新，服务器已自动重启应用最新配置");
            return Ok(());
        } else if let Some(server) = server_guard.as_ref() {
            server.apply_runtime_config(&new_config).await;
            log::info!("代理配置已实时应用，无需重启代理服务器");
        }

        Ok(())
    }

    /// 检查服务器是否正在运行。
    pub async fn is_running(&self) -> bool {
        self.server.read().await.is_some()
    }

    /// 热更新运行中服务器的全部熔断器配置。
    pub async fn update_circuit_breaker_configs(
        &self,
        config: crate::proxy::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server.update_circuit_breaker_configs(config).await;
            log::info!("已热更新运行中的熔断器配置");
        } else {
            log::debug!("代理服务器未运行，熔断器配置将在下次启动时生效");
        }
        Ok(())
    }

    /// 热更新指定应用的熔断器配置。
    pub async fn update_circuit_breaker_config_for_app(
        &self,
        app_type: &str,
        config: crate::proxy::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server
                .update_circuit_breaker_config_for_app(app_type, config)
                .await;
        }
        Ok(())
    }
}
