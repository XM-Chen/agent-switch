//! 托盘菜单管理。
//!
//! 独立网关模式下，托盘只展示进程内网关状态并控制启停。这里不得读取 Provider
//! 当前态、客户端 Live 配置、接管快照或客户端凭据。

use tauri::menu::{Menu, MenuBuilder, MenuItem};
use tauri::Manager;

use crate::error::AppError;
use crate::store::AppState;

pub const TRAY_ID: &str = "cc-switch";

#[derive(Clone, Copy)]
struct TrayTexts {
    show_main: &'static str,
    gateway_running: &'static str,
    gateway_stopped: &'static str,
    start_gateway: &'static str,
    stop_gateway: &'static str,
    quit: &'static str,
}

impl TrayTexts {
    fn from_language(language: &str) -> Self {
        match language {
            "en" => Self {
                show_main: "Open main window",
                gateway_running: "Gateway: Running",
                gateway_stopped: "Gateway: Stopped",
                start_gateway: "Start gateway",
                stop_gateway: "Stop gateway",
                quit: "Quit",
            },
            "ja" => Self {
                show_main: "メインウィンドウを開く",
                gateway_running: "ゲートウェイ：実行中",
                gateway_stopped: "ゲートウェイ：停止中",
                start_gateway: "ゲートウェイを起動",
                stop_gateway: "ゲートウェイを停止",
                quit: "終了",
            },
            "zh-TW" => Self {
                show_main: "開啟主介面",
                gateway_running: "閘道：執行中",
                gateway_stopped: "閘道：已停止",
                start_gateway: "啟動閘道",
                stop_gateway: "停止閘道",
                quit: "退出",
            },
            _ => Self {
                show_main: "打开主界面",
                gateway_running: "网关：运行中",
                gateway_stopped: "网关：已停止",
                start_gateway: "启动网关",
                stop_gateway: "停止网关",
                quit: "退出",
            },
        }
    }
}

pub fn create_tray_menu(
    app: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<Menu<tauri::Wry>, AppError> {
    let settings = crate::settings::get_settings();
    let texts = TrayTexts::from_language(settings.language.as_deref().unwrap_or("zh"));
    let running = futures::executor::block_on(app_state.proxy_service.is_running());

    let show_main = MenuItem::with_id(app, "show_main", texts.show_main, true, None::<&str>)
        .map_err(|e| AppError::Message(format!("创建打开主界面菜单失败: {e}")))?;
    let status_label = if running {
        texts.gateway_running
    } else {
        texts.gateway_stopped
    };
    let gateway_status =
        MenuItem::with_id(app, "gateway_status", status_label, false, None::<&str>)
            .map_err(|e| AppError::Message(format!("创建网关状态菜单失败: {e}")))?;
    let start_gateway = MenuItem::with_id(
        app,
        "gateway_start",
        texts.start_gateway,
        !running,
        None::<&str>,
    )
    .map_err(|e| AppError::Message(format!("创建启动网关菜单失败: {e}")))?;
    let stop_gateway = MenuItem::with_id(
        app,
        "gateway_stop",
        texts.stop_gateway,
        running,
        None::<&str>,
    )
    .map_err(|e| AppError::Message(format!("创建停止网关菜单失败: {e}")))?;
    let quit = MenuItem::with_id(app, "quit", texts.quit, true, None::<&str>)
        .map_err(|e| AppError::Message(format!("创建退出菜单失败: {e}")))?;

    MenuBuilder::new(app)
        .item(&show_main)
        .separator()
        .item(&gateway_status)
        .item(&start_gateway)
        .item(&stop_gateway)
        .separator()
        .item(&quit)
        .build()
        .map_err(|e| AppError::Message(format!("构建托盘菜单失败: {e}")))
}

pub fn refresh_tray_menu(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match create_tray_menu(app, state.inner()) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                if let Err(error) = tray.set_menu(Some(menu)) {
                    log::error!("刷新托盘菜单失败: {error}");
                }
            }
        }
        Err(error) => log::error!("创建托盘菜单失败: {error}"),
    }
}

#[cfg(target_os = "macos")]
pub fn apply_tray_policy(app: &tauri::AppHandle, dock_visible: bool) {
    use tauri::ActivationPolicy;

    let desired_policy = if dock_visible {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    };

    if let Err(error) = app.set_dock_visibility(dock_visible) {
        log::warn!("设置 Dock 显示状态失败: {error}");
    }
    if let Err(error) = app.set_activation_policy(desired_policy) {
        log::warn!("设置激活策略失败: {error}");
    }
}

pub fn handle_tray_menu_event(app: &tauri::AppHandle, event_id: &str) {
    match event_id {
        "show_main" => show_main_window(app),
        "gateway_start" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = {
                    let state = app.state::<AppState>();
                    state.proxy_service.start().await
                };
                if let Err(error) = result {
                    log::error!("从托盘启动网关失败: {error}");
                }
                refresh_tray_menu(&app);
            });
        }
        "gateway_stop" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = {
                    let state = app.state::<AppState>();
                    if state.proxy_service.is_running().await {
                        state.proxy_service.stop().await
                    } else {
                        Ok(())
                    }
                };
                if let Err(error) = result {
                    log::error!("从托盘停止网关失败: {error}");
                }
                refresh_tray_menu(&app);
            });
        }
        "quit" => app.exit(0),
        "gateway_status" => {}
        _ => log::warn!("未处理的托盘菜单事件: {event_id}"),
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "windows")]
        {
            let _ = window.set_skip_taskbar(false);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        #[cfg(target_os = "linux")]
        {
            crate::linux_fix::nudge_main_window(window.clone());
        }
        #[cfg(target_os = "macos")]
        {
            apply_tray_policy(app, true);
        }
    } else if crate::lightweight::is_lightweight_mode() {
        if let Err(error) = crate::lightweight::exit_lightweight_mode(app) {
            log::error!("退出轻量模式重建窗口失败: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TRAY_ID;

    #[test]
    fn tray_id_is_unique_to_app() {
        assert_eq!(TRAY_ID, "cc-switch");
        assert_ne!(TRAY_ID, "main");
    }
}
