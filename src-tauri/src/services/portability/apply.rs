//! 导入应用：replace / merge 策略 + 事务写入。
//!
//! - replace（full_backup）：事务内各表先 DELETE 再按包内容 INSERT，保留原 id（同环境恢复）。
//! - merge（portable）：新 UUID + old_id→new_id 重映射（endpoint.account_id /
//!   alias.target_endpoint_id / model.endpoint_id），按匹配键 upsert 非敏感字段，
//!   不动本机已有 api_key/凭据，未命中则新增。
//! - tool_takeover：两种模式均强制 enabled=0，绝不写 Claude Code/Codex 配置。
//! - 永不导入：request_logs、model_locks、tool_takeover_backups、测试数据。
//!
//! 全程在一个 SQLite 事务内，任一步失败 ROLLBACK，不留半成品。

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;

use super::package::{Payload, PORTABLE_METADATA_KEYS};
use crate::services::crypto::b64_decode;

/// 导入结果统计。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub accounts: usize,
    pub endpoints: usize,
    pub endpoint_models: usize,
    pub model_aliases: usize,
    pub route_settings: usize,
    pub tool_takeover: usize,
}

/// 应用策略。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApplyStrategy {
    /// 完整备份：replace，保留原 id。
    Replace,
    /// 脱敏迁移：merge，新 UUID + 重映射。
    Merge,
}

/// 在单个事务内应用 payload。
///
/// 调用方负责在导入前对 full_backup 做本地 DB 文件备份。
pub fn apply(
    conn: &Mutex<Connection>,
    payload: &Payload,
    strategy: ApplyStrategy,
) -> Result<ImportReport, String> {
    let mut guard = conn.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let tx = guard
        .transaction()
        .map_err(|e| format!("开启事务失败: {}", e))?;

    let report = match strategy {
        ApplyStrategy::Replace => apply_replace(&tx, payload),
        ApplyStrategy::Merge => apply_merge(&tx, payload),
    };

    match report {
        Ok(r) => {
            tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;
            Ok(r)
        }
        Err(e) => {
            // ROLLBACK 由 Drop 处理，这里显式返回错误。
            let _ = tx.rollback();
            Err(e)
        }
    }
}

// ── replace（full_backup）──────────────────────────────────────────────────

fn apply_replace(tx: &rusqlite::Transaction<'_>, p: &Payload) -> Result<ImportReport, String> {
    // 顺序：先账号 → 端点 → 模型 → 别名 → 路由 → 接管（接管强制关闭）。
    // 端点引用 account_id，需先建账号。模型/别名引用 endpoint_id，需先建端点。

    // 账号：保留原 id，连凭据 BLOB 一起恢复。
    tx.execute("DELETE FROM accounts", [])
        .map_err(|e| format!("清空账号失败: {}", e))?;
    for a in &p.accounts {
        let creds_blob = decode_blob_opt(&a.credentials_b64)?;
        tx.execute(
            "INSERT INTO accounts (id, name, account_type, platform, status, credentials_encrypted, extra_json, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![
                a.id, a.name, a.account_type, a.platform, a.status, creds_blob, a.extra_json, a.priority, now_iso()?,
            ],
        )
        .map_err(|e| format!("插入账号失败: {}", e))?;
    }

    // 端点：保留原 id，连 api_key BLOB 一起恢复。
    tx.execute("DELETE FROM endpoints", [])
        .map_err(|e| format!("清空端点失败: {}", e))?;
    for ep in &p.endpoints {
        let key_blob = decode_blob_opt(&ep.api_key_b64)?;
        tx.execute(
            "INSERT INTO endpoints (id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, extra_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            rusqlite::params![
                ep.id,
                ep.account_id,
                ep.name,
                ep.base_url,
                ep.protocol_type,
                key_blob,
                ep.auth_mode,
                ep.enabled as i64,
                ep.priority,
                ep.extra_json,
                now_iso()?,
            ],
        )
        .map_err(|e| format!("插入端点失败: {}", e))?;
    }

    // 模型：保留原 id。
    tx.execute("DELETE FROM endpoint_models", [])
        .map_err(|e| format!("清空模型失败: {}", e))?;
    for m in &p.endpoint_models {
        tx.execute(
            "INSERT INTO endpoint_models (id, endpoint_id, model_name, display_name, source, capabilities, context_window, is_available, last_seen_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                m.id,
                m.endpoint_id,
                m.model_name,
                m.display_name,
                m.source,
                m.capabilities,
                m.context_window,
                m.is_available as i64,
                m.last_seen_at,
                now_iso()?,
            ],
        )
        .map_err(|e| format!("插入模型失败: {}", e))?;
    }

    // 别名：保留原 id。
    tx.execute("DELETE FROM model_aliases", [])
        .map_err(|e| format!("清空别名失败: {}", e))?;
    for al in &p.model_aliases {
        tx.execute(
            "INSERT INTO model_aliases (id, scope_type, scope_id, alias_name, target_endpoint_id, target_model_name, priority, enabled, invalid_reason, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                al.id,
                al.scope_type,
                al.scope_id,
                al.alias_name,
                al.target_endpoint_id,
                al.target_model_name,
                al.priority,
                al.enabled as i64,
                al.invalid_reason,
                now_iso()?,
            ],
        )
        .map_err(|e| format!("插入别名失败: {}", e))?;
    }

    // 路由设置：replace 语义为整表覆盖，先清空再按包内容恢复。
    tx.execute("DELETE FROM route_settings", [])
        .map_err(|e| format!("清空路由设置失败: {}", e))?;
    for rs in &p.route_settings {
        tx.execute(
            "INSERT INTO route_settings (id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                rs.id,
                rs.label,
                rs.strategy,
                rs.protocol_type,
                rs.failover_enabled as i64,
                rs.max_switches,
                rs.same_account_retries,
                rs.cooldown_multiplier,
                now_iso()?,
            ],
        )
        .map_err(|e| format!("插入路由设置失败: {}", e))?;
    }

    // UI 偏好：replace 模式按白名单覆盖，包内缺失表示恢复为未设置。
    apply_ui_settings_replace(tx, &p.ui_settings)?;

    // 接管状态：强制 enabled=0，不写工具配置。
    apply_tool_takeover_replace(tx, &p.tool_takeover)?;

    Ok(ImportReport {
        accounts: p.accounts.len(),
        endpoints: p.endpoints.len(),
        endpoint_models: p.endpoint_models.len(),
        model_aliases: p.model_aliases.len(),
        route_settings: p.route_settings.len(),
        tool_takeover: p.tool_takeover.len(),
    })
}

/// replace 模式下写入接管状态：清空原状态表，按包写入但全部 enabled=0。
fn apply_tool_takeover_replace(
    tx: &rusqlite::Transaction<'_>,
    tools: &[super::package::ToolTakeoverExport],
) -> Result<(), String> {
    // replace 语义：整表覆盖。包内仅保留 enabled 标记（无 last_* 字段），
    // 故清空后只插入 (tool, enabled=0)，last_* 置空。
    tx.execute("DELETE FROM tool_takeover", [])
        .map_err(|e| format!("清空接管状态失败: {}", e))?;
    for t in tools {
        tx.execute(
            "INSERT INTO tool_takeover (tool, enabled, updated_at) VALUES (?1, 0, ?2)",
            rusqlite::params![t.tool, now_iso()?],
        )
        .map_err(|e| format!("写入接管状态失败: {}", e))?;
    }
    Ok(())
}

/// replace 模式下写入 UI 偏好：按白名单清空并恢复包内值。
///
/// 白名单外的 app_metadata 键保持不变（不影响运行状态键）。
fn apply_ui_settings_replace(
    tx: &rusqlite::Transaction<'_>,
    ui_settings: &[(String, String)],
) -> Result<(), String> {
    let now = now_iso()?;
    // 删除白名单偏好键（包内未提供时视为恢复未设置）。
    for key in PORTABLE_METADATA_KEYS {
        tx.execute(
            "DELETE FROM app_metadata WHERE key = ?1",
            rusqlite::params![key],
        )
        .map_err(|e| format!("清空偏好键失败: {}", e))?;
    }
    // 插入包内提供的偏好键。
    for (k, v) in ui_settings {
        if !PORTABLE_METADATA_KEYS.contains(&k.as_str()) {
            tracing::warn!("导入包含非白名单偏好键 '{}'，已跳过", k);
            continue;
        }
        tx.execute(
            "INSERT INTO app_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![k, v, now],
        )
        .map_err(|e| format!("写入偏好设置失败: {}", e))?;
    }
    Ok(())
}

// ── merge（portable）───────────────────────────────────────────────────────

fn apply_merge(tx: &rusqlite::Transaction<'_>, p: &Payload) -> Result<ImportReport, String> {
    let now = now_iso()?;
    let mut report = ImportReport::default();

    // 1. 账号：按 name + account_type + platform 匹配，命中更新非敏感字段，未命中新增（新 UUID）。
    //    新增时建 old_id→new_id 映射（供 endpoint.account_id 重映射）。
    let mut account_map: HashMap<String, String> = HashMap::new();
    for a in &p.accounts {
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM accounts WHERE name = ?1 AND account_type = ?2 AND platform = ?3 LIMIT 1",
                rusqlite::params![a.name, a.account_type, a.platform],
                |row| row.get(0),
            )
            .ok();

        match existing_id {
            Some(eid) => {
                // 命中：更新非敏感字段（name/type/platform/priority/extra_json），不动凭据与状态。
                tx.execute(
                    "UPDATE accounts SET name=?1, account_type=?2, platform=?3, priority=?4, extra_json=COALESCE(?5, extra_json), updated_at=?6 WHERE id=?7",
                    rusqlite::params![a.name, a.account_type, a.platform, a.priority, a.extra_json, now, eid],
                )
                .map_err(|e| format!("更新账号失败: {}", e))?;
                account_map.insert(a.id.clone(), eid);
            }
            None => {
                // 未命中：新增，新 UUID，凭据缺失（merge 不导入凭据）。
                let new_id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO accounts (id, name, account_type, platform, status, credentials_encrypted, extra_json, priority, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?8)",
                    rusqlite::params![new_id, a.name, a.account_type, a.platform, a.status, a.extra_json, a.priority, now],
                )
                .map_err(|e| format!("插入账号失败: {}", e))?;
                account_map.insert(a.id.clone(), new_id);
            }
        }
        report.accounts += 1;
    }

    // 2. 端点：按 name + base_url + protocol_type 匹配，命中更新非敏感字段（不动 api_key），
    //    未命中新增（新 UUID）。account_id 按 account_map 重映射。
    let mut endpoint_map: HashMap<String, String> = HashMap::new();
    for ep in &p.endpoints {
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM endpoints WHERE name = ?1 AND base_url = ?2 AND protocol_type = ?3 LIMIT 1",
                rusqlite::params![ep.name, ep.base_url, ep.protocol_type],
                |row| row.get(0),
            )
            .ok();

        let remapped_account = ep
            .account_id
            .as_ref()
            .and_then(|aid| account_map.get(aid).cloned());

        match existing_id {
            Some(eid) => {
                // 命中：更新非敏感字段，绝不覆盖本机已有 api_key_encrypted。
                tx.execute(
                    "UPDATE endpoints SET name=?1, base_url=?2, protocol_type=?3, auth_mode=?4, priority=?5,
                       account_id=COALESCE(?6, account_id), extra_json=COALESCE(?7, extra_json), updated_at=?8 WHERE id=?9",
                    rusqlite::params![
                        ep.name, ep.base_url, ep.protocol_type, ep.auth_mode, ep.priority,
                        remapped_account, ep.extra_json, now, eid,
                    ],
                )
                .map_err(|e| format!("更新端点失败: {}", e))?;
                endpoint_map.insert(ep.id.clone(), eid);
            }
            None => {
                // 未命中：新增，新 UUID，api_key 缺失（merge 不导入凭据）。
                let new_id = uuid::Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO endpoints (id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, extra_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        new_id, remapped_account, ep.name, ep.base_url, ep.protocol_type,
                        ep.auth_mode, ep.enabled as i64, ep.priority, ep.extra_json, now,
                    ],
                )
                .map_err(|e| format!("插入端点失败: {}", e))?;
                endpoint_map.insert(ep.id.clone(), new_id);
            }
        }
        report.endpoints += 1;
    }

    // 3. 模型：按 endpoint_id（重映射后）+ model_name upsert。
    for m in &p.endpoint_models {
        let endpoint_id = match endpoint_map.get(&m.endpoint_id) {
            Some(eid) => eid.clone(),
            // 端点未导入（可能被匹配跳过），用原 id 兜底，但若端点不存在则跳过该模型。
            None => continue,
        };
        let new_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO endpoint_models (id, endpoint_id, model_name, display_name, source, capabilities, context_window, is_available, last_seen_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(endpoint_id, model_name) DO UPDATE SET
               display_name=excluded.display_name, capabilities=excluded.capabilities,
               context_window=excluded.context_window, is_available=excluded.is_available,
               last_seen_at=excluded.last_seen_at, updated_at=excluded.updated_at",
            rusqlite::params![
                new_id, endpoint_id, m.model_name, m.display_name, m.source,
                m.capabilities, m.context_window, m.is_available as i64, m.last_seen_at, now,
            ],
        )
        .map_err(|e| format!("upsert 模型失败: {}", e))?;
        report.endpoint_models += 1;
    }

    // 4. 别名：按 scope_type + scope_id + alias_name upsert。target_endpoint_id 重映射。
    for al in &p.model_aliases {
        let remapped_target = al
            .target_endpoint_id
            .as_ref()
            .and_then(|tid| endpoint_map.get(tid).cloned());
        let new_id = uuid::Uuid::new_v4().to_string();
        // scope_id 为 NULL 时的匹配：用 IS 处理。
        let existing_id: Option<String> = if let Some(sid) = &al.scope_id {
            tx.query_row(
                "SELECT id FROM model_aliases WHERE scope_type=?1 AND scope_id=?2 AND alias_name=?3 LIMIT 1",
                rusqlite::params![al.scope_type, sid, al.alias_name],
                |row| row.get(0),
            )
            .ok()
        } else {
            tx.query_row(
                "SELECT id FROM model_aliases WHERE scope_type=?1 AND scope_id IS NULL AND alias_name=?2 LIMIT 1",
                rusqlite::params![al.scope_type, al.alias_name],
                |row| row.get(0),
            )
            .ok()
        };

        match existing_id {
            Some(eid) => {
                // 命中：更新非敏感字段，target_endpoint_id 重映射（若映射存在则更新，否则保留原值）。
                if let Some(rt) = &remapped_target {
                    tx.execute(
                        "UPDATE model_aliases SET scope_type=?1, alias_name=?2, target_endpoint_id=?3, target_model_name=?4, priority=?5, enabled=?6, invalid_reason=?7, updated_at=?8 WHERE id=?9",
                        rusqlite::params![al.scope_type, al.alias_name, rt, al.target_model_name, al.priority, al.enabled as i64, al.invalid_reason, now, eid],
                    )
                    .map_err(|e| format!("更新别名失败: {}", e))?;
                } else {
                    tx.execute(
                        "UPDATE model_aliases SET scope_type=?1, alias_name=?2, target_model_name=?3, priority=?4, enabled=?5, invalid_reason=?6, updated_at=?7 WHERE id=?8",
                        rusqlite::params![al.scope_type, al.alias_name, al.target_model_name, al.priority, al.enabled as i64, al.invalid_reason, now, eid],
                    )
                    .map_err(|e| format!("更新别名失败: {}", e))?;
                }
            }
            None => {
                tx.execute(
                    "INSERT INTO model_aliases (id, scope_type, scope_id, alias_name, target_endpoint_id, target_model_name, priority, enabled, invalid_reason, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        new_id, al.scope_type, al.scope_id, al.alias_name, remapped_target,
                        al.target_model_name, al.priority, al.enabled as i64, al.invalid_reason, now,
                    ],
                )
                .map_err(|e| format!("插入别名失败: {}", e))?;
            }
        }
        report.model_aliases += 1;
    }

    // 5. 路由设置：按 id upsert（claude-code/codex/v1）。
    for rs in &p.route_settings {
        tx.execute(
            "INSERT INTO route_settings (id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               label=excluded.label, strategy=excluded.strategy, protocol_type=excluded.protocol_type,
               failover_enabled=excluded.failover_enabled, max_switches=excluded.max_switches,
               same_account_retries=excluded.same_account_retries, cooldown_multiplier=excluded.cooldown_multiplier,
               updated_at=excluded.updated_at",
            rusqlite::params![
                rs.id, rs.label, rs.strategy, rs.protocol_type,
                rs.failover_enabled as i64, rs.max_switches, rs.same_account_retries, rs.cooldown_multiplier, now,
            ],
        )
        .map_err(|e| format!("upsert 路由设置失败: {}", e))?;
        report.route_settings += 1;
    }

    // 6. ui_settings：按白名单 upsert app_metadata 偏好键。
    for (k, v) in &p.ui_settings {
        if !PORTABLE_METADATA_KEYS.contains(&k.as_str()) {
            tracing::warn!("导入包含非白名单偏好键 '{}'，已跳过", k);
            continue;
        }
        tx.execute(
            "INSERT INTO app_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
            rusqlite::params![k, v, now],
        )
        .map_err(|e| format!("写入偏好设置失败: {}", e))?;
    }

    // 7. 接管状态：强制 enabled=0（merge 模式同样强制）。
    for t in &p.tool_takeover {
        tx.execute(
            "INSERT INTO tool_takeover (tool, enabled, updated_at)
             VALUES (?1, 0, ?2)
             ON CONFLICT(tool) DO UPDATE SET enabled=0, updated_at=excluded.updated_at",
            rusqlite::params![t.tool, now],
        )
        .map_err(|e| format!("写入接管状态失败: {}", e))?;
        report.tool_takeover += 1;
    }

    Ok(report)
}

// ── helpers ────────────────────────────────────────────────────────────────

/// 解码可选的 base64 BLOB（凭据）。full_backup 模式用于恢复加密 BLOB。
fn decode_blob_opt(b64: &Option<String>) -> Result<Option<Vec<u8>>, String> {
    match b64 {
        None => Ok(None),
        Some(s) => Ok(Some(b64_decode(s)?)),
    }
}

fn now_iso() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}
