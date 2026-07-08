mod app_state;
mod commands;
mod config;
mod db;
mod http;
mod services;

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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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

            // 升级回填：把存量 tool_takeover.enabled=1 的 tool 桥接为默认 proxy provider。
            // 必须在迁移后（providers 表已建）+ AppState 构造前运行；失败按迁移失败同等处理。
            let backfill =
                db::dao::providers::backfill_from_takeover(db.as_ref()).unwrap_or_else(|e| {
                    tracing::error!("升级回填失败: {}", e);
                    panic!("升级回填失败: {}", e);
                });
            tracing::info!(
                created = backfill.created,
                skipped_existing_current = backfill.skipped_existing_current,
                skipped_takeover_disabled = backfill.skipped_takeover_disabled,
                "升级回填完成"
            );

            // 首次启动自动导入 CLAUDE.md（cc-prompts，幂等：DB 非空跳过）。
            // 失败仅告警不阻断启动（与 model_sync 启动刷新错误风格一致）。
            match services::prompts::claude::import_on_first_launch(db.as_ref()) {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!("首次启动已自动导入 CLAUDE.md（{} 项）", n);
                    }
                }
                Err(e) => tracing::warn!("首次自动导入 CLAUDE.md 失败: {}", e),
            }

            // 初始化凭据加密服务。
            // Keychain 不可用时为 None，应用仍可启动，但凭据相关功能进入降级模式。
            let crypto = match services::keychain::ensure_master_key() {
                Ok(key) => {
                    tracing::info!("凭据加密服务已就绪");
                    Some(Arc::new(services::crypto::CryptoService::new(key)))
                }
                Err(e) => {
                    tracing::warn!("系统凭据管理器不可用，凭据功能进入降级模式: {}", e);
                    None
                }
            };

            let codex_oauth = Arc::new(services::codex_oauth::CodexOAuthService::new());
            let model_sync = Arc::new(services::model_sync::ModelSyncService::new());

            // Create the shared shutdown channel
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

            let web_dist_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");

            // 初始化协议转换器注册表（new() 已注册 4 个内置转换器 + Passthrough）
            use services::translator::TranslatorRegistry;
            let registry = Arc::new(TranslatorRegistry::new());

            // 初始化代理转发服务
            let route_proxy = Arc::new(http::proxy::RouteProxy::new(
                db.clone(),
                registry,
                crypto.clone(),
                codex_oauth.clone(),
                data_dir.clone(),
            ));

            let app_state = Arc::new(app_state::AppState {
                db: db.clone(),
                shutdown_tx: tokio::sync::Mutex::new(Some(shutdown_tx)),
                data_dir,
                web_dist_dir,
                crypto,
                codex_oauth,
                model_sync,
                route_proxy: tokio::sync::RwLock::new(Some(route_proxy)),
            });

            // Register as managed state
            handle.manage(app_state.clone());

            // 如果启用自动刷新，启动后异步执行一次模型刷新。
            let sync_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                if services::model_sync::ModelSyncService::is_auto_refresh_enabled(&sync_state.db) {
                    tracing::info!("自动刷新已启用，启动时刷新上游模型");
                    if let Err(e) = sync_state.model_sync.sync_all(sync_state.clone()).await {
                        tracing::warn!("启动模型刷新失败: {}", e);
                    }
                }
            });

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
