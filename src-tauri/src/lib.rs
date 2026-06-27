mod app_state;
mod commands;
mod config;
mod db;
mod http;

use std::sync::Arc;
use tauri::Manager;
use tokio::sync::oneshot;

/// Tauri application setup and entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agent_switch=info".into()),
        )
        .init();

    tracing::info!("Agent-Switch 正在启动...");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Resolve data directory
            let data_dir = handle
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| config::paths::app_data_dir());
            std::fs::create_dir_all(&data_dir).ok();
            let db_path = config::paths::db_path(&data_dir);
            tracing::info!("数据目录：{}", data_dir.display());

            // Initialize database
            let db = db::connection::open_db(&db_path)
                .map_err(|e| {
                    tracing::error!("数据库初始化失败: {}", e);
                    e
                })
                .expect("数据库初始化失败，应用无法启动");

            db::migrations::run_migrations(db.as_ref()).unwrap_or_else(|e| {
                tracing::error!("迁移执行失败: {}", e);
                panic!("数据库迁移失败: {}", e);
            });

            // Create the shared shutdown channel
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

            let web_dist_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");

            let app_state = Arc::new(app_state::AppState {
                db: db.clone(),
                shutdown_tx: tokio::sync::Mutex::new(Some(shutdown_tx)),
                data_dir,
                web_dist_dir,
            });

            // Register as managed state
            handle.manage(app_state.clone());

            // Start HTTP server with the shutdown receiver
            let http_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = http::start_server(http_state, shutdown_rx).await {
                    tracing::error!("HTTP 服务异常退出: {}", e);
                }
            });

            // Listen for window close to trigger graceful shutdown
            let close_state = app_state.clone();
            let window = app.get_webview_window("main").unwrap();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    let state = close_state.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut guard = state.shutdown_tx.lock().await;
                        if let Some(tx) = guard.take() {
                            let _ = tx.send(()).ok();
                        }
                    });
                    tracing::info!("应用窗口已关闭，本地服务停止。");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::app::get_app_info])
        .run(tauri::generate_context!())
        .expect("Agent-Switch 启动失败");
}
