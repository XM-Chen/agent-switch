use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::init_status::InitErrorPayload;

/// 打开 GatewayShell 中的外部文档链接。
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<bool, String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{url}")
    };

    app.opener()
        .open_url(&url, None::<String>)
        .map_err(|e| format!("打开链接失败: {e}"))?;
    Ok(true)
}

/// 打开 Agent Switch 自身的配置目录（仅项目数据，不触达客户端配置）。
#[tauri::command]
pub async fn open_app_config_folder(app: AppHandle) -> Result<bool, String> {
    let path = crate::config::get_app_config_dir();
    app.opener()
        .open_path(path.to_string_lossy().as_ref(), None::<String>)
        .map_err(|e| format!("打开配置目录失败: {e}"))?;
    Ok(true)
}

/// 获取启动阶段的初始化错误。
#[tauri::command]
pub async fn get_init_error() -> Result<Option<InitErrorPayload>, String> {
    Ok(crate::init_status::get_init_error())
}

/// 设置管理窗口主题。
#[tauri::command]
pub async fn set_window_theme(window: tauri::Window, theme: String) -> Result<(), String> {
    let tauri_theme = match theme.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    window.set_theme(tauri_theme).map_err(|e| e.to_string())
}
