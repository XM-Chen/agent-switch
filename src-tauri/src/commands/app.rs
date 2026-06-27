use serde::Serialize;
use std::sync::Arc;

use crate::app_state::AppState;

/// Basic application information returned by Tauri commands.
#[derive(Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub data_dir: String,
}

/// Tauri command: returns basic application metadata.
#[tauri::command]
pub fn get_app_info(state: tauri::State<'_, Arc<AppState>>) -> AppInfo {
    AppInfo {
        name: "agent-switch",
        version: "0.1.0",
        data_dir: state.data_dir.display().to_string(),
    }
}
