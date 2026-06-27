pub mod error;
pub mod health;
pub mod placeholders;
pub mod router;

use axum::Router;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::app_state::AppState;

/// Start the Axum server on `127.0.0.1:42567`.
///
/// `shutdown_rx` is the receiver for graceful shutdown.
pub async fn start_server(
    state: Arc<AppState>,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), String> {
    let app: Router = router::build(state);

    let addr = "127.0.0.1:42567";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("无法绑定 {}: {}", addr, e))?;

    tracing::info!("本地服务已启动：http://{}/", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
            tracing::info!("本地服务正在关闭...");
        })
        .await
        .map_err(|e| format!("服务运行失败: {}", e))?;

    tracing::info!("本地服务已停止");
    Ok(())
}
