//! 导出数据收集：从各表读取 → `Payload`。
//!
//! - `full_backup`：凭据列原样装入加密 BLOB 的 base64（不解密重装，减少明文暴露面）。
//! - `portable`：凭据列置 None；剔除 request_logs / 测试数据 / 媒体 / 接管备份文件；
//!   ui_settings 仅取白名单偏好键。
//!
//! 不导出的表：request_logs、model_locks、tool_takeover_backups（备份文件本机专属）。

use rusqlite::Connection;
use std::sync::Mutex;

use super::package::{
    AccountExport, AliasExport, EndpointExport, ModelExport, Payload, RouteSettingExport,
    ToolTakeoverExport, PORTABLE_METADATA_KEYS,
};

use crate::services::crypto::b64_encode;

/// 收集模式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CollectMode {
    FullBackup,
    Portable,
}

/// 从当前库收集全部可导出数据，组装为 `Payload`。
pub fn collect(db: &Mutex<Connection>, mode: CollectMode) -> Result<Payload, String> {
    let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let accounts = collect_accounts(&conn, mode)?;
    let endpoints = collect_endpoints(&conn, mode)?;
    let endpoint_models = collect_models(&conn)?;
    let model_aliases = collect_aliases(&conn)?;
    let route_settings = collect_route_settings(&conn)?;
    let tool_takeover = collect_tool_takeover(&conn)?;
    let ui_settings = collect_ui_settings(&conn)?;

    Ok(Payload {
        accounts,
        endpoints,
        endpoint_models,
        model_aliases,
        route_settings,
        tool_takeover,
        ui_settings,
    })
}

fn collect_accounts(conn: &Connection, mode: CollectMode) -> Result<Vec<AccountExport>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, account_type, platform, status, credentials_encrypted, extra_json, priority
             FROM accounts ORDER BY priority ASC, created_at ASC",
        )
        .map_err(|e| format!("查询账号失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let creds_blob: Option<Vec<u8>> = row.get("credentials_encrypted")?;
            Ok(AccountExport {
                id: row.get("id")?,
                name: row.get("name")?,
                account_type: row.get("account_type")?,
                platform: row.get("platform")?,
                status: row.get("status")?,
                credentials_b64: match mode {
                    // full_backup 装入已加密 BLOB；portable 脱敏置 None。
                    CollectMode::FullBackup => creds_blob.as_deref().map(b64_encode),
                    CollectMode::Portable => None,
                },
                extra_json: row.get("extra_json")?,
                priority: row.get("priority")?,
            })
        })
        .map_err(|e| format!("读取账号失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("账号行解析失败: {}", e))?);
    }
    Ok(out)
}

fn collect_endpoints(conn: &Connection, mode: CollectMode) -> Result<Vec<EndpointExport>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, extra_json
             FROM endpoints ORDER BY priority ASC, created_at ASC",
        )
        .map_err(|e| format!("查询端点失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let key_blob: Option<Vec<u8>> = row.get("api_key_encrypted")?;
            let enabled: i64 = row.get("enabled")?;
            Ok(EndpointExport {
                id: row.get("id")?,
                account_id: row.get("account_id")?,
                name: row.get("name")?,
                base_url: row.get("base_url")?,
                protocol_type: row.get("protocol_type")?,
                api_key_b64: match mode {
                    // full_backup 装入已加密 BLOB；portable 脱敏置 None。
                    CollectMode::FullBackup => key_blob.as_deref().map(b64_encode),
                    CollectMode::Portable => None,
                },
                auth_mode: row.get("auth_mode")?,
                enabled: enabled != 0,
                priority: row.get("priority")?,
                extra_json: row.get("extra_json")?,
            })
        })
        .map_err(|e| format!("读取端点失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("端点行解析失败: {}", e))?);
    }
    Ok(out)
}

fn collect_models(conn: &Connection) -> Result<Vec<ModelExport>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, endpoint_id, model_name, display_name, source, capabilities, context_window, is_available, last_seen_at
             FROM endpoint_models ORDER BY endpoint_id, model_name",
        )
        .map_err(|e| format!("查询模型失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let is_available: i64 = row.get("is_available")?;
            Ok(ModelExport {
                id: row.get("id")?,
                endpoint_id: row.get("endpoint_id")?,
                model_name: row.get("model_name")?,
                display_name: row.get("display_name")?,
                source: row.get("source")?,
                capabilities: row.get("capabilities")?,
                context_window: row.get("context_window")?,
                is_available: is_available != 0,
                last_seen_at: row.get("last_seen_at")?,
            })
        })
        .map_err(|e| format!("读取模型失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("模型行解析失败: {}", e))?);
    }
    Ok(out)
}

fn collect_aliases(conn: &Connection) -> Result<Vec<AliasExport>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, scope_type, scope_id, alias_name, target_endpoint_id, target_model_name, priority, enabled, invalid_reason
             FROM model_aliases ORDER BY scope_type, scope_id, alias_name, priority",
        )
        .map_err(|e| format!("查询别名失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let enabled: i64 = row.get("enabled")?;
            Ok(AliasExport {
                id: row.get("id")?,
                scope_type: row.get("scope_type")?,
                scope_id: row.get("scope_id")?,
                alias_name: row.get("alias_name")?,
                target_endpoint_id: row.get("target_endpoint_id")?,
                target_model_name: row.get("target_model_name")?,
                priority: row.get("priority")?,
                enabled: enabled != 0,
                invalid_reason: row.get("invalid_reason")?,
            })
        })
        .map_err(|e| format!("读取别名失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("别名行解析失败: {}", e))?);
    }
    Ok(out)
}

fn collect_route_settings(conn: &Connection) -> Result<Vec<RouteSettingExport>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier
             FROM route_settings ORDER BY id",
        )
        .map_err(|e| format!("查询路由设置失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let failover_enabled: i64 = row.get("failover_enabled")?;
            Ok(RouteSettingExport {
                id: row.get("id")?,
                label: row.get("label")?,
                strategy: row.get("strategy")?,
                protocol_type: row.get("protocol_type")?,
                failover_enabled: failover_enabled != 0,
                max_switches: row.get("max_switches")?,
                same_account_retries: row.get("same_account_retries")?,
                cooldown_multiplier: row.get("cooldown_multiplier")?,
            })
        })
        .map_err(|e| format!("读取路由设置失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("路由设置行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 工具接管状态：仅保留 was_enabled 标记，导入后强制关闭。
fn collect_tool_takeover(conn: &Connection) -> Result<Vec<ToolTakeoverExport>, String> {
    let mut stmt = conn
        .prepare("SELECT tool, enabled FROM tool_takeover ORDER BY tool")
        .map_err(|e| format!("查询接管状态失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let enabled: i64 = row.get("enabled")?;
            Ok(ToolTakeoverExport {
                tool: row.get("tool")?,
                was_enabled: enabled != 0,
            })
        })
        .map_err(|e| format!("读取接管状态失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("接管状态行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 收集 ui_settings：仅白名单偏好键（如 `auto_model_refresh_enabled`）。
///
/// 排除本机运行状态键（`last_model_sync_at` / `last_model_sync_error`）。
fn collect_ui_settings(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for key in PORTABLE_METADATA_KEYS {
        let mut stmt = conn
            .prepare("SELECT value FROM app_metadata WHERE key = ?1")
            .map_err(|e| format!("查询设置失败: {}", e))?;
        let mut rows = stmt
            .query_map(rusqlite::params![key], |row| row.get::<_, String>(0))
            .map_err(|e| format!("读取设置失败: {}", e))?;
        if let Some(r) = rows.next() {
            let v = r.map_err(|e| format!("设置行解析失败: {}", e))?;
            out.push((key.to_string(), v));
        }
    }
    Ok(out)
}
