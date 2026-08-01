// 阶段 5：客户端配置/接管/迁移模块已从生产启动、IPC、托盘、网关路由中移除。
// 静态架构守卫 (tests/gateway_safety.rs) 是"未触达客户端配置"的强保证。仍保留的
// 旧数据 provenance 模块（v17 一次性迁移读取旧 providers 表所需）在各自模块内
// 显式标注 `#[allow(dead_code)]`，其余死代码必须被删除或修复。

mod app_config;
mod app_store;
mod auto_launch;
mod commands;
mod config;
mod database;
mod error;
pub mod gateway;
mod init_status;
mod lightweight;
#[cfg(target_os = "linux")]
mod linux_fix;
mod panic_hook;
mod provider;
mod provider_defaults;
mod proxy;
mod services;
mod settings;
mod store;
mod test_hooks;

mod tray;
mod usage_events;

pub use app_config::AppType;
pub use commands::*;
pub use database::{CcAggregateConfig, Database};
pub use error::AppError;
pub use provider::{Provider, ProviderMeta};
pub use services::ProxyService;
pub use settings::{
    reload_settings as reload_settings_for_test_hook, update_settings, AppSettings,
};
pub use store::AppState;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
pub use test_hooks::{
    add_provider_test_hook, delete_provider_test_hook, update_provider_test_hook,
};

use std::sync::Arc;
#[cfg(target_os = "macos")]
use tauri::image::Image;
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri::RunEvent;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

#[cfg(target_os = "windows")]
fn set_windows_app_user_model_id(app: &tauri::AppHandle) {
    let app_id = app.config().identifier.clone();
    let wide_app_id: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();

    let result = unsafe {
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(wide_app_id.as_ptr())
    };

    if result < 0 {
        log::warn!("设置 Windows AppUserModelID 失败: 0x{result:08X}");
    } else {
        log::debug!("Windows AppUserModelID 已设置为 {app_id}");
    }
}

#[tauri::command]
async fn update_tray_menu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    match tray::create_tray_menu(&app, state.inner()) {
        Ok(new_menu) => {
            if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
                tray.set_menu(Some(new_menu))
                    .map_err(|e| format!("更新托盘菜单失败: {e}"))?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(err) => {
            log::error!("创建托盘菜单失败: {err}");
            Ok(false)
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_tray_icon() -> Option<Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/tray/macos/statusbar_template_3x.png");

    match Image::from_bytes(ICON_BYTES) {
        Ok(icon) => Some(icon),
        Err(err) => {
            log::warn!("Failed to load macOS tray icon: {err}");
            None
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置 panic hook，在应用崩溃时记录日志到 <app_config_dir>/crash.log（默认 ~/.agent-switch/crash.log）
    panic_hook::setup_panic_hook();

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            log::info!("=== Single Instance Callback Triggered ===");
            log::debug!("Args count: {}", args.len());

            if crate::lightweight::is_lightweight_mode() {
                if let Err(e) = crate::lightweight::exit_lightweight_mode(app) {
                    log::error!("退出轻量模式重建窗口失败: {e}");
                }
            }

            // Check for deep link URL in args (mainly for Windows/Linux command line)
            let _ = &args;

            // Show and focus window regardless
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
                #[cfg(target_os = "linux")]
                {
                    linux_fix::nudge_main_window(window.clone());
                }
            }
        }));
    }

    let builder = builder
        // 拦截窗口关闭：根据设置决定是否最小化到托盘
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 数据库版本过新的恢复模式下没有托盘可唤回，关闭即退出，避免应用隐身后台
                let in_db_recovery = crate::init_status::get_init_error()
                    .map(|p| p.kind.as_deref() == Some("db_version_too_new"))
                    .unwrap_or(false);
                if in_db_recovery {
                    api.prevent_close();
                    window.app_handle().exit(0);
                    return;
                }

                let settings = crate::settings::get_settings();

                if settings.minimize_to_tray_on_close {
                    api.prevent_close();
                    let _ = window.hide();
                    #[cfg(target_os = "windows")]
                    {
                        let _ = window.set_skip_taskbar(true);
                    }
                    #[cfg(target_os = "macos")]
                    {
                        tray::apply_tray_policy(window.app_handle(), false);
                    }
                } else {
                    api.prevent_close();
                    window.app_handle().exit(0);
                }
            }
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .setup(|app| {
            let _ = rustls::crypto::ring::default_provider().install_default();

            // 预先刷新 Store 覆盖配置，确保后续路径读取正确（日志/数据库等）
            app_store::refresh_app_config_dir_override(app.handle());
            panic_hook::init_app_config_dir(crate::config::get_app_config_dir());
            #[cfg(target_os = "windows")]
            set_windows_app_user_model_id(app.handle());

            // 注册 Updater 插件（桌面端）
            #[cfg(desktop)]
            {
                if let Err(e) = app
                    .handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())
                {
                    // 若配置不完整（如缺少 pubkey），跳过 Updater 而不中断应用
                    log::warn!("初始化 Updater 插件失败，已跳过：{e}");
                }
            }
            // 初始化日志（单文件输出到 <app_config_dir>/logs/agent-switch.log）
            {
                use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

                let log_dir = panic_hook::get_log_dir();

                // 确保日志目录存在
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("创建日志目录失败: {e}");
                }

                // 启动时删除旧日志文件，实现单文件覆盖效果
                let log_file_path = log_dir.join("agent-switch.log");
                let _ = std::fs::remove_file(&log_file_path);

                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        // 初始化为 Trace，允许后续通过 log::set_max_level() 动态调整级别
                        .level(log::LevelFilter::Trace)
                        .targets([
                            Target::new(TargetKind::Stdout),
                            Target::new(TargetKind::Folder {
                                path: log_dir,
                                file_name: Some("agent-switch".into()),
                            }),
                        ])
                        // 单文件模式：启动时删除旧文件，达到大小时轮转
                        // 注意：KeepSome(n) 内部会做 n-2 运算，n=1 会导致 usize 下溢
                        // KeepSome(2) 是最小安全值，表示不保留轮转文件
                        .rotation_strategy(RotationStrategy::KeepSome(2))
                        // 单文件大小限制 1GB
                        .max_file_size(1024 * 1024 * 1024)
                        .timezone_strategy(TimezoneStrategy::UseLocal)
                        .build(),
                )?;
            }

            // 注入 AppHandle 给 usage_events，让无 AppHandle 持有的写日志路径
            // 也能向前端推送 `usage-log-recorded`。
            // 放在日志系统初始化之后，确保 init 的日志能正常输出。
            usage_events::init(app.handle().clone());

            // 只初始化 Agent Switch 自身数据库。
            //
            // 旧版 config.json 迁移已停用：历史 loader 会进一步探测客户端 Prompt、
            // Skills 与 Live 配置，不符合独立网关的零客户端文件访问边界。旧文件原样
            // 保留，后续如需迁移必须由只解析 Agent Switch 数据的显式工具完成。
            let app_config_dir = crate::config::get_app_config_dir();
            let db_path = app_config_dir.join("agent-switch.db");

            // 现在创建数据库（包含 Schema 迁移）
            //
            // 说明：从 v3.8.* 升级的用户通常会走到这里的 SQLite schema 迁移，
            // 若迁移失败（数据库损坏/权限不足/user_version 过新等），需要给用户明确提示，
            // 否则表现可能只是“应用打不开/闪退”。
            //
            // 预检：数据库版本过新时，必须先于任何 schema 写操作（create_tables 内含
            // DROP/ALTER 等 DDL）进入恢复界面，避免旧应用对读不懂的更新版 DB 落写。
            match crate::database::Database::stored_user_version_exceeds_supported(&db_path) {
                Ok(Some(version)) => {
                    log::warn!("数据库版本过新（v{version}），引导用户在应用内升级应用");
                    crate::init_status::set_init_error(crate::init_status::InitErrorPayload {
                        path: db_path.display().to_string(),
                        error: format!(
                            "数据库版本过新（{version}），当前应用仅支持 {}，请升级应用后再尝试。",
                            crate::database::SCHEMA_VERSION
                        ),
                        kind: Some("db_version_too_new".to_string()),
                        db_version: Some(version),
                        supported_version: Some(crate::database::SCHEMA_VERSION),
                    });
                    // 主窗口默认 visible:false，恢复界面必须强制显示
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    return Ok(());
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("预检数据库版本失败，继续正常初始化流程: {e}");
                }
            }

            let db = loop {
                match crate::database::Database::init() {
                    Ok(db) => break Arc::new(db),
                    Err(e) => {
                        log::error!("Failed to init database: {e}");

                        if !show_database_init_error_dialog(app.handle(), &db_path, &e.to_string())
                        {
                            log::info!("用户选择退出程序");
                            std::process::exit(1);
                        }

                        log::info!("用户选择重试初始化数据库");
                    }
                }
            };

            let app_state = AppState::new(db);

            match crate::services::gateway_auth::GatewayAuthService::migrate_legacy_token(
                &app_state.db,
            ) {
                Ok(true) => log::info!("✓ 已将旧网关 token 迁移为不可逆哈希"),
                Ok(false) => {}
                Err(error) => log::error!("迁移旧网关 token 失败: {error}"),
            }
            if let Err(error) = app_state.db.set_gateway_auth_required(true) {
                log::error!("强制启用独立网关鉴权失败: {error}");
            }

            // 网关运行事件仍通过 AppHandle 通知前端；不再启动或连接任何
            // 客户端配置监控器。
            app_state.proxy_service.set_app_handle(app.handle().clone());

            // 官方上游模板仅写入 Agent Switch 自身数据库，不读取客户端配置。
            match app_state.db.init_default_official_providers() {
                Ok(count) if count > 0 => {
                    log::info!("✓ Seeded {count} official upstream template(s)");
                }
                Ok(_) => {}
                Err(e) => log::warn!("✗ Failed to seed official upstream templates: {e}"),
            }

            // 迁移旧的 app_config_dir 配置到 Store
            if let Err(e) = app_store::migrate_app_config_dir_from_settings(app.handle()) {
                log::warn!("迁移 app_config_dir 失败: {e}");
            }

            // 启动阶段不再无条件保存,避免意外覆盖用户配置。

            // 创建动态托盘菜单
            let menu = tray::create_tray_menu(app.handle(), &app_state)?;

            // 构建托盘
            let mut tray_builder = TrayIconBuilder::with_id(tray::TRAY_ID)
                .tooltip("Agent-Switch") // 鼠标悬停提示
                .menu(&menu)
                .on_menu_event(|app, event| {
                    tray::handle_tray_menu_event(app, &event.id.0);
                })
                .show_menu_on_left_click(true);

            // 使用平台对应的托盘图标（macOS 使用模板图标适配深浅色）
            #[cfg(target_os = "macos")]
            {
                if let Some(icon) = macos_tray_icon() {
                    tray_builder = tray_builder.icon(icon).icon_as_template(true);
                } else if let Some(icon) = app.default_window_icon() {
                    log::warn!("Falling back to default window icon for tray");
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to load macOS tray icon for tray");
                }
            }

            #[cfg(not(target_os = "macos"))]
            {
                if let Some(icon) = app.default_window_icon() {
                    tray_builder = tray_builder.icon(icon.clone());
                } else {
                    log::warn!("Failed to get default window icon for tray");
                }
            }

            let _tray = tray_builder.build(app)?;
            // 将同一个实例注入到全局状态，避免重复创建导致的不一致。
            // WebDAV/S3 自动同步在完成“仅网关数据”审计前保持停用，防止旧导入链
            // 通过 post-import hook 间接投影到客户端 Live 配置。
            app.manage(app_state);

            // 从数据库加载日志配置并应用
            {
                let db = &app.state::<AppState>().db;
                if let Ok(log_config) = db.get_log_config() {
                    log::set_max_level(log_config.to_level_filter());
                    log::info!(
                        "已加载日志配置: enabled={}, level={}",
                        log_config.enabled,
                        log_config.level
                    );
                }
            }

            // 初始化 CopilotAuthManager
            {
                use crate::proxy::providers::copilot_auth::CopilotAuthManager;
                use crate::proxy::auth_state::CopilotAuthState;
                use tokio::sync::RwLock;

                let app_config_dir = crate::config::get_app_config_dir();
                let copilot_auth_manager = CopilotAuthManager::new(app_config_dir);
                app.manage(CopilotAuthState(Arc::new(RwLock::new(copilot_auth_manager))));
                log::info!("✓ CopilotAuthManager initialized");
            }

            // 初始化 CodexOAuthManager (ChatGPT Plus/Pro 反代)
            {
                use crate::proxy::providers::codex_oauth_auth::CodexOAuthManager;
                use crate::proxy::auth_state::CodexOAuthState;
                use tokio::sync::RwLock;

                let app_config_dir = crate::config::get_app_config_dir();
                let codex_oauth_manager = CodexOAuthManager::new(app_config_dir);
                app.manage(CodexOAuthState(Arc::new(RwLock::new(codex_oauth_manager))));
                log::info!("✓ CodexOAuthManager initialized");
            }

            // 初始化全局出站代理 HTTP 客户端
            {
                let db = &app.state::<AppState>().db;
                let proxy_url = db.get_global_proxy_url().ok().flatten();

                if let Err(e) = crate::proxy::http_client::init(proxy_url.as_deref()) {
                    log::error!(
                        "[GlobalProxy] [GP-005] Failed to initialize with saved config: {e}"
                    );

                    // 清除无效的代理配置
                    if proxy_url.is_some() {
                        log::warn!(
                            "[GlobalProxy] [GP-006] Clearing invalid proxy config from database"
                        );
                        if let Err(clear_err) = db.set_global_proxy_url(None) {
                            log::error!(
                                "[GlobalProxy] [GP-007] Failed to clear invalid config: {clear_err}"
                            );
                        }
                    }

                    // 使用直连模式重新初始化
                    if let Err(fallback_err) = crate::proxy::http_client::init(None) {
                        log::error!(
                            "[GlobalProxy] [GP-008] Failed to initialize direct connection: {fallback_err}"
                        );
                    }
                }
            }

            // 新进程尚无 server，运行镜像必须归零。此处只访问 Agent Switch DB；
            // 不执行 crash restore、common snippet 提取、客户端会话扫描或外部配置监控。
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();
                if let Err(error) = state.db.reset_proxy_runtime_mirror().await {
                    log::error!("归零网关运行镜像失败: {error}");
                }

                // 自动整库备份暂时停用。旧库可能仍含客户端 Live 快照，复制整库会继续
                // 持有/扩散这些历史数据；阶段 5 改为仅导出纯网关表后再恢复自动备份。
            });

            // Linux: 禁用 WebKitGTK 硬件加速，防止 EGL 初始化失败导致白屏
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        use webkit2gtk::{WebViewExt, SettingsExt, HardwareAccelerationPolicy};
                        let wk_webview = webview.inner();
                        if let Some(settings) = WebViewExt::settings(&wk_webview) {
                            SettingsExt::set_hardware_acceleration_policy(&settings, HardwareAccelerationPolicy::Never);
                            log::info!("已禁用 WebKitGTK 硬件加速");
                        }
                    });
                }
            }

            // 静默启动：根据设置决定是否显示主窗口
            let settings = crate::settings::get_settings();
            if let Some(window) = app.get_webview_window("main") {
                // 在窗口首次显示前同步装饰状态，避免前端加载后再切换导致标题栏闪烁
                // 仅 Linux 生效：解决 Wayland 下系统窗口按钮不可用的问题
                #[cfg(target_os = "linux")]
                let _ = window.set_decorations(!settings.use_app_window_controls);
                if settings.silent_startup {
                    // 静默启动模式：保持窗口隐藏
                    let _ = window.hide();
                    #[cfg(target_os = "windows")]
                    let _ = window.set_skip_taskbar(true);
                    #[cfg(target_os = "macos")]
                    tray::apply_tray_policy(app.handle(), false);
                    log::info!("静默启动模式：主窗口已隐藏");
                } else {
                    // 正常启动模式：显示窗口
                    let _ = window.show();
                    log::info!("正常启动模式：主窗口已显示");

                    // Linux: 解决首次启动 UI 无响应问题（Tauri #10746 + wry #637）。
                    // 启动时 webview 未获取焦点 + surface 尺寸协商失败，导致点击无效。
                    // 这里做 set_focus + 伪 resize，等价于无视觉版本的"最大化-还原"。
                    #[cfg(target_os = "linux")]
                    {
                        linux_fix::nudge_main_window(window.clone());
                    }
                }
            }


            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // GatewayShell 基础设施
            commands::open_external,
            commands::open_app_config_folder,
            commands::get_init_error,
            commands::get_settings,
            commands::save_settings,
            commands::install_update_and_restart,
            commands::check_app_update_available,
            update_tray_menu,
            commands::set_window_theme,
            // 网关生命周期
            commands::start_proxy_server,
            commands::stop_proxy_server,
            commands::get_proxy_status,
            commands::is_proxy_running,
            commands::get_proxy_config,
            commands::update_proxy_config,
            // 网关鉴权
            commands::get_gateway_auth_status,
            commands::create_gateway_api_key,
            commands::revoke_gateway_api_key,
            commands::set_gateway_auth_required,
            // 独立网关控制面（仅访问项目数据库）
            commands::get_gateway_domain_config,
            commands::update_gateway_domain_config,
            commands::list_gateway_upstreams,
            commands::get_gateway_upstream,
            commands::create_gateway_upstream,
            commands::update_gateway_upstream,
            commands::delete_gateway_upstream,
            commands::set_gateway_upstream_enabled,
            commands::list_gateway_upstream_credential_hints,
            commands::replace_gateway_upstream_credential,
            commands::delete_gateway_upstream_credential,
            commands::list_gateway_upstream_models,
            commands::list_gateway_models,
            commands::create_gateway_model,
            commands::update_gateway_model,
            commands::delete_gateway_model,
            commands::set_gateway_model_enabled,
            commands::set_gateway_model_state,
            commands::list_gateway_model_aliases,
            commands::upsert_gateway_model_alias,
            commands::delete_gateway_model_alias,
            commands::list_gateway_routes,
            commands::create_gateway_route_target,
            commands::update_gateway_route_target,
            commands::delete_gateway_route_target,
            commands::set_gateway_route_enabled,
            commands::set_gateway_route_target_enabled,
            commands::reorder_gateway_routes,
            commands::reorder_gateway_route_targets,
            commands::list_gateway_route_health,
            commands::list_gateway_migration_issues,
            // 本机纯网关回滚（scope/hash/DPAPI/外键均 fail-closed 验证）
            commands::import_local_gateway_rollback,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        // 处理退出请求（所有平台）
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            match classify_exit_request(*code) {
                // code 为 None 表示运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活窗口），
                // 此时应仅阻止退出、保持托盘后台运行。
                ExitRequestAction::StayInTray => {
                    log::info!("运行时触发退出请求（无存活窗口），阻止退出以保持托盘后台运行");
                    api.prevent_exit();
                    return;
                }
                // code 为 RESTART_EXIT_CODE：app.restart() / 自更新 relaunch 发起的重启。
                // 这条路径上 prevent_exit() 会被 Tauri 忽略，事件循环必定退出，随后由
                // Tauri 在 RunEvent::Exit 后用新二进制 re-exec（macOS 会按更新后的
                // Info.plist 解析可执行名）。
                //
                // 绝不能复用下面的异步清理任务：该任务在 tokio 线程调 save_window_state，
                // 持有 window-state 插件锁的同时向主线程查询窗口几何；而主线程此刻正在
                // 退出事件循环，并在插件自带的 RunEvent::Exit 钩子里等待同一把锁——双方
                // 互等造成进程永久卡死（更新已安装但应用冻结、不再重启，见 #3998）。
                //
                // 重启路径交还 Tauri 默认流程即可：
                //   - 窗口状态：插件 Exit 钩子在主线程保存（同线程读取窗口几何，无死锁）
                //   - 托盘图标：Tauri 内部 cleanup_before_exit 清理，正常走 Drop
                //   - 网关：重启会终止当前监听器，新实例按用户操作重新启动
                //   - 100ms 落盘等待：重启前的 DB 写入均为命令驱动、此刻已完成，
                //     与所有 Tauri 应用默认重启路径的行为一致，无需额外等待
                ExitRequestAction::DeferToTauriRestart => {
                    log::info!("收到重启请求 (code={code:?})，交由 Tauri 默认重启流程 re-exec");
                    return;
                }
                // 其它 Some(_)：用户主动调用 app.exit() 退出（如托盘菜单"退出"），
                // 此时执行清理后退出。
                ExitRequestAction::CleanupAndExit => {}
            }

            log::info!("收到用户主动退出请求 (code={code:?})，开始清理...");
            api.prevent_exit();

            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                save_window_state_before_exit(&app_handle);
                cleanup_before_exit(&app_handle).await;
                // 先于 std::process::exit 显式移除托盘图标。
                // 进程直接退出时 Tauri 运行时不走正常 Drop 流程，
                // 不会向 Windows Shell 发送 NIM_DELETE，导致已退出的进程
                // 注册的图标仍残留在系统托盘（鼠标悬停 Shell 才会重绘发现进程已死）。
                remove_tray_icon_before_exit(&app_handle);
                log::info!("清理完成，退出应用");

                // 短暂等待确保所有 I/O 操作（如数据库写入）刷新到磁盘
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // 使用 std::process::exit 避免再次触发 ExitRequested
                std::process::exit(0);
            });
            return;
        }

        #[cfg(target_os = "macos")]
        {
            match event {
                // macOS 在 Dock 图标被点击并重新激活应用时会触发 Reopen 事件，这里手动恢复主窗口
                RunEvent::Reopen { .. } => {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        #[cfg(target_os = "windows")]
                        {
                            let _ = window.set_skip_taskbar(false);
                        }
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                        tray::apply_tray_policy(app_handle, true);
                    } else if crate::lightweight::is_lightweight_mode() {
                        if let Err(e) = crate::lightweight::exit_lightweight_mode(app_handle) {
                            log::error!("退出轻量模式重建窗口失败: {e}");
                        }
                    }
                }
                _ => {}
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, event);
        }
    });
}

// ============================================================
// 应用退出清理
// ============================================================

/// 应用退出时只停止本进程内的网关并归零运行镜像。
/// 客户端配置、接管快照和外部文件监控均不属于退出清理边界。
pub async fn cleanup_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<store::AppState>() {
        if state.proxy_service.is_running().await {
            if let Err(error) = state.proxy_service.stop().await {
                log::error!("退出时停止网关失败: {error}");
            }
        } else if let Err(error) = state.db.reset_proxy_runtime_mirror().await {
            log::error!("退出时归零网关运行镜像失败: {error}");
        }
    }
}

/// 主动从系统托盘移除托盘图标。
///
/// `std::process::exit` 会绕过 Tauri 运行时，触发不了 `TrayIcon::drop()`，
/// 也就不会向 Windows Shell 发 `NIM_DELETE`。结果是进程退出后托盘里
/// 仍保留一个死图标的缓存占位（Shell 不会主动重绘，需要鼠标悬停才刷新）。
///
/// 通过 `set_visible(false)` 走 `WM_USER_HIDE_TRAYICON` 消息路径，
/// 触发 tray-icon 内部的 `remove_tray_icon` → `Shell_NotifyIconW(NIM_DELETE)`，
/// 在进程结束前干净地把图标摘掉。其它平台 `set_visible(false)` 也是
/// 正常的隐藏/移除语义，作为跨平台兜底也安全。
pub(crate) fn remove_tray_icon_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(tray) = app_handle.tray_by_id(tray::TRAY_ID) {
        if let Err(e) = tray.set_visible(false) {
            log::warn!("退出时移除托盘图标失败: {e}");
        } else {
            log::info!("已显式从系统托盘移除图标");
        }
    }
}

// ============================================================
// 数据库初始化错误对话框
// ============================================================

/// 检测是否为中文环境
fn is_chinese_locale() -> bool {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .map(|lang| lang.starts_with("zh"))
        .unwrap_or(false)
}

/// 显示数据库初始化/Schema 迁移失败对话框
/// 返回 true 表示用户选择重试，false 表示用户选择退出
fn show_database_init_error_dialog(
    app: &tauri::AppHandle,
    db_path: &std::path::Path,
    error: &str,
) -> bool {
    let title = if is_chinese_locale() {
        "数据库初始化失败"
    } else {
        "Database Initialization Failed"
    };

    let message = if is_chinese_locale() {
        format!(
            "初始化数据库或迁移数据库结构时发生错误：\n\n{error}\n\n\
            数据库文件路径：\n{db}\n\n\
            您的数据尚未丢失，应用不会自动删除数据库文件。\n\
            常见原因包括：数据库版本过新、文件损坏、权限不足、磁盘空间不足等。\n\n\
            建议：\n\
            1) 先备份整个配置目录（包含 agent-switch.db）\n\
            2) 如果提示“数据库版本过新”，请升级到更新版本\n\
            3) 如果刚升级出现异常，可回退旧版本导出/备份后再升级\n\n\
            点击「重试」重新尝试初始化\n\
            点击「退出」关闭程序",
            db = db_path.display()
        )
    } else {
        format!(
            "An error occurred while initializing or migrating the database:\n\n{error}\n\n\
            Database file path:\n{db}\n\n\
            Your data is NOT lost - the app will not delete the database automatically.\n\
            Common causes include: newer database version, corrupted file, permission issues, or low disk space.\n\n\
            Suggestions:\n\
            1) Back up the entire config directory (including agent-switch.db)\n\
            2) If you see “database version is newer”, please upgrade Agent Switch\n\
            3) If this happened right after upgrading, consider rolling back to export/backup then upgrade again\n\n\
            Click 'Retry' to attempt initialization again\n\
            Click 'Exit' to close the program",
            db = db_path.display()
        )
    };

    let retry_text = if is_chinese_locale() {
        "重试"
    } else {
        "Retry"
    };
    let exit_text = if is_chinese_locale() {
        "退出"
    } else {
        "Exit"
    };

    app.dialog()
        .message(&message)
        .title(title)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            retry_text.to_string(),
            exit_text.to_string(),
        ))
        .blocking_show()
}

// ============================================================
// 退出请求分类
// ============================================================

/// `RunEvent::ExitRequested` 的三类来源，处理方式必须区分。
///
/// 关键约束：重启请求（`code == RESTART_EXIT_CODE`）上 `prevent_exit()` 会被
/// Tauri 静默忽略（见 `ExitRequestApi::prevent_exit` 文档），事件循环必定继续
/// 退出并触发各插件的 `RunEvent::Exit` 钩子；任何与之并发的自定义清理任务都
/// 可能与插件退出钩子争用同一状态而死锁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    /// `code` 为 `None`：运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活
    /// 窗口），阻止退出、保持托盘后台运行。
    StayInTray,
    /// `code` 为 `RESTART_EXIT_CODE`：`app.restart()` / 自更新 relaunch 发起的
    /// 重启，不拦截、不做自定义清理，交还 Tauri 默认 re-exec 流程。
    DeferToTauriRestart,
    /// 其它 `Some(_)`：用户主动退出（托盘「退出」等），执行完整异步清理后结束进程。
    CleanupAndExit,
}

fn classify_exit_request(code: Option<i32>) -> ExitRequestAction {
    match code {
        None => ExitRequestAction::StayInTray,
        Some(tauri::RESTART_EXIT_CODE) => ExitRequestAction::DeferToTauriRestart,
        Some(_) => ExitRequestAction::CleanupAndExit,
    }
}

// ============================================================
// 在应用主动退出前显式持久化窗口状态
// ============================================================

fn window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

/// 当前应用的退出路径会拦截 `ExitRequested` 并最终直接 `std::process::exit(0)`，
/// 这里需要在真正结束进程前手动落盘，避免 window-state 插件的默认退出钩子被绕过。
pub fn save_window_state_before_exit(app_handle: &tauri::AppHandle) {
    if let Err(err) = app_handle.save_window_state(window_state_flags()) {
        log::error!("退出前保存窗口状态失败: {err}");
    } else {
        log::info!("已在退出前保存窗口状态");
    }
}

/// 主动释放 single-instance 锁。
///
/// macOS single-instance 使用 `/tmp/{identifier}.sock`。我们有若干路径会直接
/// `std::process::exit(0)`，不会触发插件挂在 `RunEvent::Exit` 上的清理钩子。
/// 重启前主动 destroy 可以避免新进程误连旧 listener 后自行退出。
pub fn destroy_single_instance_lock(app_handle: &tauri::AppHandle) {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    tauri_plugin_single_instance::destroy(app_handle);
}

/// 清理托盘图标、释放 single-instance 锁后重启当前应用。
///
/// 直接走 `tauri::process::restart`（spawn 新进程 + `exit(0)`），不经过事件
/// 循环退出，因此 Tauri 内部的 `cleanup_before_exit` 和各插件的
/// `RunEvent::Exit` 钩子都不会执行。需要的清理由调用方与本函数显式补偿：
/// 窗口状态、代理/Live 恢复（调用方）；托盘图标、single-instance 锁（本函数）。
///
/// 有意不调 `AppHandle::cleanup_before_exit()`：它会在调用线程上 Drop 托盘
/// 图标，而 macOS 的 NSStatusItem 操作要求主线程；`set_visible(false)` 走
/// `run_item_main_thread` 代理，跨线程安全（见 `remove_tray_icon_before_exit`）。
pub fn restart_process(app_handle: &tauri::AppHandle) -> ! {
    remove_tray_icon_before_exit(app_handle);
    destroy_single_instance_lock(app_handle);
    tauri::process::restart(&app_handle.env());
}

#[cfg(test)]
mod tests {
    use super::{classify_exit_request, ExitRequestAction};

    #[test]
    fn no_code_keeps_app_alive_in_tray() {
        assert_eq!(classify_exit_request(None), ExitRequestAction::StayInTray);
    }

    #[test]
    fn restart_exit_code_defers_to_tauri_default_restart() {
        assert_eq!(
            classify_exit_request(Some(tauri::RESTART_EXIT_CODE)),
            ExitRequestAction::DeferToTauriRestart
        );
    }

    #[test]
    fn user_exit_codes_run_cleanup_then_exit() {
        assert_eq!(
            classify_exit_request(Some(0)),
            ExitRequestAction::CleanupAndExit
        );
        assert_eq!(
            classify_exit_request(Some(1)),
            ExitRequestAction::CleanupAndExit
        );
    }
}
