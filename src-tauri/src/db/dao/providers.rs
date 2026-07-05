// 本模块的 DAO 函数将在后续子任务（双模式接管 / Provider CRUD API）逐步接线。
// 在接线完成前标记为 allow(dead_code)，与 route_settings.rs 的先例一致。
#![allow(dead_code)]

use rusqlite::{params, Connection};
use std::sync::Mutex;

use super::now_iso;
use crate::services::provider::AppType;
use crate::services::tool_takeover::Tool;

/// Provider 行（数据库原始表示）。
///
/// `providers` 是 ccs 式「可切换单元」，是切换面（UI 选哪个、当前激活哪个）。
/// 与现有 `endpoints`+`accounts` 并存：proxy 模式 provider 把工具指向本地代理，
/// 上游路由仍由 endpoints 管道决定；direct 模式 provider 的 settings_config
/// 内含真实配置，直接写工具文件绕过代理。
#[derive(Debug, Clone)]
pub struct ProviderRow {
    pub id: String,
    pub app_type: String,
    pub name: String,
    /// 'proxy' | 'direct'
    pub mode: String,
    /// JSON：工具原生配置或代理指向配置。
    pub settings_config: String,
    pub is_current: bool,
    /// official/third_party/aggregator/custom（对齐 ccs）。
    pub category: Option<String>,
    pub sort_index: Option<i64>,
    pub notes: Option<String>,
    /// JSON：不写入 live 的元数据。
    pub meta: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建 provider 的输入。
#[derive(Debug, Clone)]
pub struct NewProvider {
    pub id: String,
    pub app_type: String,
    pub name: String,
    pub mode: String,
    pub settings_config: String,
    pub category: Option<String>,
    pub sort_index: Option<i64>,
    pub notes: Option<String>,
    pub meta: String,
}

/// 更新 provider 的输入（部分字段）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
/// `is_current` 不在此更新——激活态互斥必须走 `set_current` 事务。
#[derive(Debug, Clone, Default)]
pub struct ProviderUpdate {
    pub name: Option<String>,
    pub mode: Option<String>,
    pub settings_config: Option<String>,
    pub category: Option<Option<String>>,
    pub sort_index: Option<Option<i64>>,
    pub notes: Option<Option<String>>,
    pub meta: Option<String>,
}

fn row_to_provider(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderRow> {
    Ok(ProviderRow {
        id: row.get("id")?,
        app_type: row.get("app_type")?,
        name: row.get("name")?,
        mode: row.get("mode")?,
        settings_config: row.get("settings_config")?,
        is_current: row.get::<_, i64>("is_current")? != 0,
        category: row.get("category")?,
        sort_index: row.get("sort_index")?,
        notes: row.get("notes")?,
        meta: row.get("meta")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 列出指定 app_type 的全部 provider，按 sort_index 升序（NULL 排最后）、再按创建时间。
pub fn list_by_app(db: &Mutex<Connection>, app_type: &str) -> Result<Vec<ProviderRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare(
            "SELECT * FROM providers WHERE app_type = ?1
             ORDER BY sort_index IS NULL, sort_index ASC, created_at ASC",
        )
        .map_err(|e| format!("查询 provider 失败: {}", e))?;
    let rows = stmt
        .query_map(params![app_type], row_to_provider)
        .map_err(|e| format!("读取 provider 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("provider 行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 获取单个 provider。
pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<ProviderRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM providers WHERE id = ?1")
        .map_err(|e| format!("查询 provider 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_provider)
        .map_err(|e| format!("读取 provider 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("provider 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 获取指定 app_type 当前激活的 provider。
pub fn get_current(db: &Mutex<Connection>, app_type: &str) -> Result<Option<ProviderRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM providers WHERE app_type = ?1 AND is_current = 1")
        .map_err(|e| format!("查询当前 provider 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![app_type], row_to_provider)
        .map_err(|e| format!("读取 provider 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("provider 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 计算指定 app_type 的下一个 sort_index（MAX+1，追加到末尾）。
pub fn next_sort_index(db: &Mutex<Connection>, app_type: &str) -> Result<i64, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let max: Option<i64> = db
        .query_row(
            "SELECT MAX(sort_index) FROM providers WHERE app_type = ?1",
            params![app_type],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询 sort_index 失败: {}", e))?;
    Ok(max.map(|m| m + 1).unwrap_or(0))
}

/// 创建 provider（不激活；is_current 恒为 0）。
pub fn create(db: &Mutex<Connection>, new: NewProvider) -> Result<ProviderRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO providers (id, app_type, name, mode, settings_config, is_current, category, sort_index, notes, meta, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                new.id,
                new.app_type,
                new.name,
                new.mode,
                new.settings_config,
                new.category,
                new.sort_index,
                new.notes,
                new.meta,
                now,
            ],
        )
        .map_err(|e| format!("创建 provider 失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取 provider".to_string())
}

/// 更新 provider 部分字段。不改 is_current（走 set_current）。
pub fn update(db: &Mutex<Connection>, id: &str, upd: ProviderUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(v) = &upd.name {
        sets.push("name = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &upd.mode {
        sets.push("mode = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &upd.settings_config {
        sets.push("settings_config = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(opt) = &upd.category {
        sets.push("category = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &upd.sort_index {
        sets.push("sort_index = ?".to_string());
        params_vec.push(Box::new(*opt));
    }
    if let Some(opt) = &upd.notes {
        sets.push("notes = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(v) = &upd.meta {
        sets.push("meta = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".to_string());
    params_vec.push(Box::new(now));
    params_vec.push(Box::new(id.to_string()));

    let sql = format!("UPDATE providers SET {} WHERE id = ?", sets.join(", "));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    db.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("更新 provider 失败: {}", e))?;
    Ok(())
}

/// 将指定 provider 设为其 app_type 的当前激活项（互斥）。
///
/// 事务内先把同 app_type 全部清零，再置目标为 1，保证每 app_type 至多一个 current。
/// DB 层的 partial unique index (`idx_providers_current`) 兜底并发写入。
pub fn set_current(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let now = now_iso()?;
    let mut guard = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let tx = guard
        .transaction()
        .map_err(|e| format!("开启事务失败: {}", e))?;

    // 查出目标的 app_type（同时校验存在）。
    let app_type: String = tx
        .query_row(
            "SELECT app_type FROM providers WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| format!("provider 不存在或查询失败: {}", e))?;

    // 先清同 app_type 的 current，再置目标。顺序不可颠倒，否则触发唯一索引冲突。
    tx.execute(
        "UPDATE providers SET is_current = 0, updated_at = ?2
         WHERE app_type = ?1 AND is_current = 1",
        params![app_type, now],
    )
    .map_err(|e| format!("清除旧 current 失败: {}", e))?;

    tx.execute(
        "UPDATE providers SET is_current = 1, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )
    .map_err(|e| format!("设置 current 失败: {}", e))?;

    tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;
    Ok(())
}

/// 清除指定 app_type 的当前激活项（全部置 0）。
pub fn clear_current(db: &Mutex<Connection>, app_type: &str) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "UPDATE providers SET is_current = 0, updated_at = ?2
         WHERE app_type = ?1 AND is_current = 1",
        params![app_type, now],
    )
    .map_err(|e| format!("清除 current 失败: {}", e))?;
    Ok(())
}

/// 批量更新 sort_index。
pub fn update_sort_order(db: &Mutex<Connection>, updates: &[(String, i64)]) -> Result<(), String> {
    let now = now_iso()?;
    let mut guard = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let tx = guard
        .transaction()
        .map_err(|e| format!("开启事务失败: {}", e))?;
    for (id, sort_index) in updates {
        tx.execute(
            "UPDATE providers SET sort_index = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, sort_index, now],
        )
        .map_err(|e| format!("更新 sort_index 失败: {}", e))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;
    Ok(())
}

/// 删除 provider。
pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM providers WHERE id = ?1", params![id])
        .map_err(|e| format!("删除 provider 失败: {}", e))?;
    Ok(())
}

/// 升级回填结果统计。
///
/// 用于一次性把存量 `tool_takeover.enabled=1` 的用户桥接到 providers 表。
/// 字段含义：`created` 本次新建的回填行数；`skipped_existing_current` 因目标 app_type
/// 已有 `is_current=1` 行而被跳过的次数；`skipped_takeover_disabled` 因对应 tool
/// 未启用接管而被跳过的次数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillReport {
    pub created: usize,
    pub skipped_existing_current: usize,
    pub skipped_takeover_disabled: usize,
}

/// 升级回填：把存量 `tool_takeover.enabled=1` 的 tool 桥接为默认 proxy provider。
///
/// 触发场景：v7 之前用户已通过旧 API 写入 `tool_takeover.enabled=1`，但 `providers`
/// 表为空。本函数在迁移后一次性运行，为每个启用 tool 造一行确定性 id 的
/// `mode=proxy`/`is_current=1` provider，使 `/api/providers` 列表非空、UI 可显示。
///
/// 契约（详见 design.md）：
/// - 对每个 tool（claude-code / codex）：
///   1. `tool_takeover::get_state` 查 enabled；非 enabled=1 跳过
///      （`skipped_takeover_disabled` +1）。
///   2. `providers::get_current` 查是否已有 current；有则不覆盖
///      （`skipped_existing_current` +1，尊重用户已配置的 current）。
///   3. 否则用 `providers::create` 造一行确定性 id `prov-backfill-<tool>`。
///   4. 若该 id 已存在（用户保留了回填行但改了字段）→ 不覆盖，视为已完成。
/// - 幂等：可重复调用，二次运行 `created=0`。
/// - 不动 `tool_takeover` 状态、不调 `tool_takeover::enable`、不写工具配置文件。
/// - tool→app_type 映射复用 `services::tool_takeover::Tool` 与 `services::provider::AppType`，
///   避免跨表标识漂移。
pub fn backfill_from_takeover(db: &Mutex<Connection>) -> Result<BackfillReport, String> {
    let mut report = BackfillReport::default();

    // 仅回填支持接管的 tool（claude-code / codex）。OpenCode 当前无接管语义，跳过。
    let tools = [Tool::ClaudeCode, Tool::Codex];

    for tool in tools {
        let tool_str = tool.as_str();

        // 1. 查 tool_takeover 启用状态；未启用 → 跳过。
        let state = crate::db::dao::tool_takeover::get_state(db, tool_str)?;
        let enabled = state.as_ref().map(|s| s.enabled).unwrap_or(false);
        if !enabled {
            report.skipped_takeover_disabled += 1;
            continue;
        }

        // 2. 查该 app_type 是否已有 current provider；有则不覆盖。
        let app_type = tool_to_app_type(tool)?;
        if get_current(db, app_type.as_str())?.is_some() {
            report.skipped_existing_current += 1;
            continue;
        }

        // 3. 若确定性 id 已存在（用户保留回填行但改了字段）→ 不覆盖。
        let backfill_id = format!("prov-backfill-{}", tool_str);
        if get(db, &backfill_id)?.is_some() {
            // 视为已完成，不重复创建也不计数为 created。
            continue;
        }

        // 4. 造一行默认 proxy provider。
        let new = NewProvider {
            id: backfill_id,
            app_type: app_type.as_str().to_string(),
            name: format!("默认代理 ({})", tool_str),
            mode: "proxy".to_string(),
            settings_config: "{}".to_string(),
            category: None,
            sort_index: Some(0),
            notes: None,
            meta: "{}".to_string(),
        };
        let created = create(db, new)?;
        // 回填行需直接 is_current=1（create 内部恒置 0，互斥激活走 set_current）。
        set_current(db, &created.id)?;
        report.created += 1;
    }

    Ok(report)
}

/// tool → app_type 映射。复用 `services::tool_takeover::Tool` 与 `services::provider::AppType`，
/// 避免硬编码字符串导致跨表标识漂移（参考 `provider/mod.rs:23` 注释）。
fn tool_to_app_type(tool: Tool) -> Result<AppType, String> {
    match tool {
        Tool::ClaudeCode => Ok(AppType::ClaudeCode),
        Tool::Codex => Ok(AppType::Codex),
        // OpenCode 当前无对应 AppType，且不支持接管；调用方已过滤，这里兜底报错。
        other => Err(format!("tool {:?} 无对应 app_type", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("无法创建内存数据库");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移应成功");
        db
    }

    fn new_provider(id: &str, app_type: &str) -> NewProvider {
        NewProvider {
            id: id.to_string(),
            app_type: app_type.to_string(),
            name: format!("provider-{}", id),
            mode: "proxy".to_string(),
            settings_config: "{}".to_string(),
            category: None,
            sort_index: None,
            notes: None,
            meta: "{}".to_string(),
        }
    }

    #[test]
    fn create_defaults_mode_proxy_and_not_current() {
        let db = setup();
        let p = create(&db, new_provider("p1", "claude-code")).unwrap();
        assert_eq!(p.mode, "proxy", "mode 默认应为 proxy");
        assert!(!p.is_current, "新建 provider 不应自动激活");
        assert_eq!(p.meta, "{}");
    }

    #[test]
    fn set_current_is_exclusive_per_app_type() {
        let db = setup();
        create(&db, new_provider("p1", "claude-code")).unwrap();
        create(&db, new_provider("p2", "claude-code")).unwrap();
        create(&db, new_provider("c1", "codex")).unwrap();

        set_current(&db, "p1").unwrap();
        assert_eq!(get_current(&db, "claude-code").unwrap().unwrap().id, "p1");

        // 切到 p2，p1 应被自动清除。
        set_current(&db, "p2").unwrap();
        assert_eq!(get_current(&db, "claude-code").unwrap().unwrap().id, "p2");
        assert!(!get(&db, "p1").unwrap().unwrap().is_current);

        // 不同 app_type 的 current 互不影响。
        set_current(&db, "c1").unwrap();
        assert_eq!(get_current(&db, "codex").unwrap().unwrap().id, "c1");
        assert_eq!(get_current(&db, "claude-code").unwrap().unwrap().id, "p2");
    }

    #[test]
    fn next_sort_index_appends() {
        let db = setup();
        assert_eq!(next_sort_index(&db, "claude-code").unwrap(), 0);

        let mut np = new_provider("p1", "claude-code");
        np.sort_index = Some(0);
        create(&db, np).unwrap();
        assert_eq!(next_sort_index(&db, "claude-code").unwrap(), 1);

        let mut np2 = new_provider("p2", "claude-code");
        np2.sort_index = Some(5);
        create(&db, np2).unwrap();
        assert_eq!(next_sort_index(&db, "claude-code").unwrap(), 6);

        // 其他 app_type 独立计数。
        assert_eq!(next_sort_index(&db, "codex").unwrap(), 0);
    }

    #[test]
    fn list_by_app_orders_by_sort_index_nulls_last() {
        let db = setup();
        let mut a = new_provider("a", "claude-code");
        a.sort_index = Some(2);
        create(&db, a).unwrap();
        let mut b = new_provider("b", "claude-code");
        b.sort_index = Some(1);
        create(&db, b).unwrap();
        let c = new_provider("c", "claude-code"); // sort_index NULL
        create(&db, c).unwrap();

        let ids: Vec<String> = list_by_app(&db, "claude-code")
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec!["b", "a", "c"], "按 sort_index 升序，NULL 排最后");
    }

    #[test]
    fn update_partial_fields() {
        let db = setup();
        create(&db, new_provider("p1", "claude-code")).unwrap();
        update(
            &db,
            "p1",
            ProviderUpdate {
                mode: Some("direct".to_string()),
                category: Some(Some("official".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
        let p = get(&db, "p1").unwrap().unwrap();
        assert_eq!(p.mode, "direct");
        assert_eq!(p.category.as_deref(), Some("official"));
        assert_eq!(p.name, "provider-p1", "未指定字段不应改变");
    }

    #[test]
    fn clear_current_resets_app_type() {
        let db = setup();
        create(&db, new_provider("p1", "claude-code")).unwrap();
        set_current(&db, "p1").unwrap();
        clear_current(&db, "claude-code").unwrap();
        assert!(get_current(&db, "claude-code").unwrap().is_none());
    }

    #[test]
    fn update_sort_order_batch() {
        let db = setup();
        create(&db, new_provider("p1", "claude-code")).unwrap();
        create(&db, new_provider("p2", "claude-code")).unwrap();
        update_sort_order(&db, &[("p1".to_string(), 10), ("p2".to_string(), 5)]).unwrap();
        let ids: Vec<String> = list_by_app(&db, "claude-code")
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, vec!["p2", "p1"]);
    }

    #[test]
    fn delete_removes_provider() {
        let db = setup();
        create(&db, new_provider("p1", "claude-code")).unwrap();
        delete(&db, "p1").unwrap();
        assert!(get(&db, "p1").unwrap().is_none());
    }

    // ── backfill_from_takeover 单测 ──────────────────────────────

    use crate::db::dao::tool_takeover::upsert_state;

    /// 写一条 tool_takeover 状态行。
    fn takeover(db: &Mutex<Connection>, tool: &str, enabled: bool) {
        upsert_state(db, tool, enabled, "proxy", None, None, None, None).unwrap();
    }

    #[test]
    fn backfill_empty_takeover_creates_nothing() {
        let db = setup();
        let report = backfill_from_takeover(&db).unwrap();
        // 空 takeover：两个 tool 都未启用 → skipped_takeover_disabled=2，无创建。
        assert_eq!(report.created, 0, "空 takeover 不应创建 provider");
        assert_eq!(report.skipped_existing_current, 0);
        assert_eq!(
            report.skipped_takeover_disabled, 2,
            "两个 tool 都未启用应计入 2 次跳过"
        );
        // providers 仍应空。
        assert!(list_by_app(&db, "claude-code").unwrap().is_empty());
        assert!(list_by_app(&db, "codex").unwrap().is_empty());
    }

    #[test]
    fn backfill_enabled_takeover_creates_current_proxy_row() {
        let db = setup();
        takeover(&db, "claude-code", true);

        let report = backfill_from_takeover(&db).unwrap();
        assert_eq!(report.created, 1, "应创建 1 行");
        assert_eq!(report.skipped_existing_current, 0);
        assert_eq!(
            report.skipped_takeover_disabled, 1,
            "codex 未启用应计入跳过"
        );

        let current = get_current(&db, "claude-code").unwrap().unwrap();
        assert_eq!(current.id, "prov-backfill-claude-code");
        assert_eq!(current.app_type, "claude-code");
        assert_eq!(current.name, "默认代理 (claude-code)");
        assert_eq!(current.mode, "proxy");
        assert_eq!(current.settings_config, "{}");
        assert!(current.is_current, "回填行应为 current");
        assert_eq!(current.sort_index, Some(0));
        assert_eq!(current.meta, "{}");
    }

    #[test]
    fn backfill_skips_when_current_already_exists() {
        let db = setup();
        takeover(&db, "claude-code", true);
        // 用户已手动建过 current provider。
        create(&db, new_provider("user-p", "claude-code")).unwrap();
        set_current(&db, "user-p").unwrap();

        let report = backfill_from_takeover(&db).unwrap();
        assert_eq!(report.created, 0, "已有 current 不应再创建");
        assert_eq!(
            report.skipped_existing_current, 1,
            "应计入 skipped_existing_current"
        );

        // 不覆盖用户 current。
        let current = get_current(&db, "claude-code").unwrap().unwrap();
        assert_eq!(current.id, "user-p");
        // 回填行不应被创建。
        assert!(get(&db, "prov-backfill-claude-code").unwrap().is_none());
    }

    #[test]
    fn backfill_is_idempotent_on_second_run() {
        let db = setup();
        takeover(&db, "claude-code", true);
        takeover(&db, "codex", true);

        let first = backfill_from_takeover(&db).unwrap();
        assert_eq!(first.created, 2, "首次应创建 2 行");

        let second = backfill_from_takeover(&db).unwrap();
        assert_eq!(second.created, 0, "二次运行不应重复创建");
        assert_eq!(
            second.skipped_existing_current, 2,
            "二次运行两个 app_type 都已有 current"
        );

        // 仍各只有一行回填 provider，且为 current。
        assert_eq!(list_by_app(&db, "claude-code").unwrap().len(), 1);
        assert_eq!(list_by_app(&db, "codex").unwrap().len(), 1);
        assert_eq!(
            get_current(&db, "claude-code").unwrap().unwrap().id,
            "prov-backfill-claude-code"
        );
        assert_eq!(
            get_current(&db, "codex").unwrap().unwrap().id,
            "prov-backfill-codex"
        );
    }

    #[test]
    fn backfill_skips_disabled_takeover() {
        let db = setup();
        // 显式 enabled=0。
        takeover(&db, "claude-code", false);

        let report = backfill_from_takeover(&db).unwrap();
        assert_eq!(report.created, 0);
        assert_eq!(
            report.skipped_takeover_disabled, 2,
            "两个 tool 都未启用应计入 2 次跳过"
        );
        assert!(get(&db, "prov-backfill-claude-code").unwrap().is_none());
    }

    #[test]
    fn backfill_preserves_user_edited_backfill_row() {
        let db = setup();
        takeover(&db, "claude-code", true);
        // 用户保留了回填 id 的行，但改了名字/没设 current。
        create(
            &db,
            NewProvider {
                id: "prov-backfill-claude-code".to_string(),
                app_type: "claude-code".to_string(),
                name: "我的自定义代理".to_string(),
                mode: "proxy".to_string(),
                settings_config: "{}".to_string(),
                category: None,
                sort_index: Some(0),
                notes: None,
                meta: "{}".to_string(),
            },
        )
        .unwrap();

        let report = backfill_from_takeover(&db).unwrap();
        // 已存在同 id → 不覆盖；同时没有 current，但 id 已存在直接视为完成，
        // 既不计 created 也不计 skipped_existing_current。
        assert_eq!(report.created, 0);
        assert_eq!(report.skipped_existing_current, 0);

        let row = get(&db, "prov-backfill-claude-code").unwrap().unwrap();
        assert_eq!(row.name, "我的自定义代理", "不应覆盖用户改动");
        assert!(!row.is_current, "不应擅自激活用户保留的行");
    }
}
