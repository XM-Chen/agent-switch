//! Claude Code MCP 同步（cc-mcp，仅 Claude Code）。
//!
//! 写 `~/.claude.json` 的 `mcpServers` 字段：**全量投影** DB 里 `enabled_claude=1` 的
//! server 规范（保留 `~/.claude.json` 其它顶层键）。MCP 是全局清单，独立于 provider
//! 切换，CRUD 后即时同步。
//!
//! 移植 ccs `mcp/claude.rs` + `claude_mcp.rs`，按 agent-switch 风格改写（`Result<_, String>`，
//! 复用 `tool_takeover::atomic_write`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::db::dao::mcp_servers::{self, NewMcpServer};
use crate::services::tool_takeover::atomic_write;

use super::validation::validate_server_spec;

/// `~/.claude.json` 路径（固定 home 下，不做 override_dir）。
pub fn claude_mcp_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".claude.json"))
        .ok_or_else(|| "无法获取用户主目录".to_string())
}

// ── Windows cmd /c 包装（移植 ccs wrap_command_for_windows）─────────────────

/// 需在 Windows 上用 `cmd /c` 包装的命令（实为 .cmd 批处理，需经 cmd 执行）。
#[cfg(windows)]
const WINDOWS_WRAP_COMMANDS: &[&str] = &["npx", "npm", "yarn", "pnpm", "node", "bun", "deno"];

/// Windows：stdio 类型且 command 属于包装列表 → 改写为 `cmd /c <command> <args>`。
#[cfg(windows)]
fn wrap_command_for_windows(obj: &mut serde_json::Map<String, Value>) {
    let server_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("stdio");
    if server_type != "stdio" {
        return;
    }
    let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) else {
        return;
    };
    // 已是 cmd 的不重复包装
    if cmd.eq_ignore_ascii_case("cmd") || cmd.eq_ignore_ascii_case("cmd.exe") {
        return;
    }
    let cmd_name = Path::new(cmd)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);
    let needs_wrap = WINDOWS_WRAP_COMMANDS
        .iter()
        .any(|&c| cmd_name.eq_ignore_ascii_case(c));
    if !needs_wrap {
        return;
    }
    let original_args = obj
        .get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut new_args = vec![Value::String("/c".into()), Value::String(cmd.into())];
    new_args.extend(original_args);
    obj.insert("command".into(), Value::String("cmd".into()));
    obj.insert("args".into(), Value::Array(new_args));
}

/// 非 Windows：no-op。
#[cfg(not(windows))]
fn wrap_command_for_windows(_obj: &mut serde_json::Map<String, Value>) {}

/// 检测路径是否为 WSL 网络路径（`\\wsl$\...` / `\\wsl.localhost\...`）。
/// WSL 跑 Linux，不需要 cmd /c 包装。仅 Windows 生效。
#[cfg(windows)]
fn is_wsl_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        match prefix.kind() {
            Prefix::UNC(server, _) | Prefix::VerbatimUNC(server, _) => {
                let s = server.to_string_lossy();
                s.eq_ignore_ascii_case("wsl$") || s.eq_ignore_ascii_case("wsl.localhost")
            }
            _ => false,
        }
    } else {
        false
    }
}

#[cfg(not(windows))]
fn is_wsl_path(_path: &Path) -> bool {
    false
}

// ── 读写 live `~/.claude.json` ──────────────────────────────────────────────

fn read_live_value(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    serde_json::from_str(&content).map_err(|e| format!("解析 {} 失败: {}", path.display(), e))
}

fn write_live_value(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化失败: {}", e))?;
    atomic_write(path, &bytes)
}

// ── 全量投影同步 ────────────────────────────────────────────────────────────

/// 把 DB 里 `enabled_claude=1` 的 server 投影写入 `~/.claude.json` 的 `mcpServers` 字段。
///
/// - 只替换 `mcpServers`，保留其它顶层键。
/// - 每个 server 取 `server_config` 规范，Windows 下做 cmd /c 包装（WSL 路径跳过）。
/// - Claude 未安装时 no-op。
pub fn sync_enabled_to_claude(db: &Mutex<Connection>) -> Result<(), String> {
    sync_enabled_to_path(db, &claude_mcp_path()?)
}

/// `sync_enabled_to_claude` 的核心（接收显式 path，便于测试）。
///
/// `force`：测试用，跳过 `should_sync` 安装检测强制写（便于用临时目录测全量投影逻辑）。
pub fn sync_enabled_to_path(db: &Mutex<Connection>, path: &Path) -> Result<(), String> {
    sync_enabled_to_path_with(db, path, should_sync_for_path(path))
}

pub fn sync_enabled_to_path_with(
    db: &Mutex<Connection>,
    path: &Path,
    perform: bool,
) -> Result<(), String> {
    if !perform {
        return Ok(());
    }
    let enabled = mcp_servers::list_enabled_claude(db)?;
    let is_wsl = is_wsl_path(path);
    if is_wsl {
        tracing::debug!("检测到 WSL 路径，跳过 cmd /c 包装: {}", path.display());
    }

    let mut servers: BTreeMap<String, Value> = BTreeMap::new();
    for row in enabled {
        let mut spec: Value = serde_json::from_str(&row.server_config)
            .map_err(|e| format!("解析 server_config '{}' 失败: {}", row.id, e))?;
        let Some(obj) = spec.as_object_mut() else {
            return Err(format!("server_config '{}' 不是 JSON 对象", row.id));
        };
        if !is_wsl {
            wrap_command_for_windows(obj);
        }
        servers.insert(row.id.clone(), Value::Object(obj.clone()));
    }

    let mut root = read_live_value(path)?;
    let Some(root_obj) = root.as_object_mut() else {
        return Err(format!("{} 根不是 JSON 对象，拒绝覆盖", path.display()));
    };
    root_obj.insert(
        "mcpServers".to_string(),
        Value::Object(servers.into_iter().collect()),
    );
    write_live_value(path, &root)
}

/// Claude 是否已安装：`~/.claude.json` 存在，或其兄弟 `~/.claude` 目录存在。
///
/// 兄弟目录由 path 自身的 parent 推导（`~/.claude.json` ↔ `~/.claude`），生产语义正确、
/// 测试用临时目录时天然隔离（不看真实 home）。
fn should_sync_for_path(path: &Path) -> bool {
    if path.exists() {
        return true;
    }
    path.parent()
        .map(|p| p.join(".claude").exists())
        .unwrap_or(false)
}

// ── 反向导入 ────────────────────────────────────────────────────────────────

/// 从 live `~/.claude.json` 的 `mcpServers` 反向导入到 DB。
///
/// 已存在同 id → 仅置 `enabled_claude=true`、不覆盖富信息；不存在 → 新建（`enabled_claude=true`，
/// name=id）。单项校验失败跳过并计入 report，不中止。
pub fn import_from_claude(db: &Mutex<Connection>) -> Result<ImportReport, String> {
    import_from_path(db, &claude_mcp_path()?)
}

/// `import_from_claude` 的核心（接收显式 path，便于测试）。
pub fn import_from_path(db: &Mutex<Connection>, path: &Path) -> Result<ImportReport, String> {
    let root = read_live_value(path)?;
    let Some(map) = root.get("mcpServers").and_then(|v| v.as_object()) else {
        return Ok(ImportReport::default());
    };
    let mut report = ImportReport::default();
    for (id, spec) in map {
        if let Err(reason) = validate_server_spec(spec) {
            report.skipped.push(SkippedItem {
                id: id.clone(),
                reason,
            });
            continue;
        }
        mcp_servers::upsert(
            db,
            NewMcpServer {
                id: id.clone(),
                name: id.clone(),
                server_config: spec.to_string(),
                description: None,
                homepage: None,
                docs: None,
                tags: "[]".to_string(),
                enabled_claude: true,
            },
        )?;
        report.imported += 1;
    }
    Ok(report)
}

/// live `~/.claude.json` 的 MCP 状态摘要。
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpStatus {
    pub config_path: String,
    pub config_exists: bool,
    pub live_server_count: usize,
}

pub fn get_status() -> Result<McpStatus, String> {
    let path = claude_mcp_path()?;
    let (exists, count) = if path.exists() {
        let v = read_live_value(&path)?;
        let n = v
            .get("mcpServers")
            .and_then(|x| x.as_object())
            .map(|m| m.len())
            .unwrap_or(0);
        (true, n)
    } else {
        (false, 0)
    };
    Ok(McpStatus {
        config_path: path.to_string_lossy().to_string(),
        config_exists: exists,
        live_server_count: count,
    })
}

/// 反向导入结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: Vec<SkippedItem>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkippedItem {
    pub id: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use serde_json::json;

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("as-mcp-test-{}-{}-{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    fn make_server(db: &Mutex<Connection>, id: &str, spec: Value, enabled: bool) {
        mcp_servers::create(
            db,
            NewMcpServer {
                id: id.to_string(),
                name: id.to_string(),
                server_config: spec.to_string(),
                description: None,
                homepage: None,
                docs: None,
                tags: "[]".to_string(),
                enabled_claude: enabled,
            },
        )
        .unwrap();
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    // ── Windows 包装（移植 ccs 测试）────────────────────────────────────────

    #[cfg(windows)]
    fn wrap(spec: Value) -> Value {
        let mut obj = spec.as_object().unwrap().clone();
        wrap_command_for_windows(&mut obj);
        Value::Object(obj)
    }

    #[cfg(not(windows))]
    fn wrap(spec: Value) -> Value {
        let mut obj = spec.as_object().unwrap().clone();
        wrap_command_for_windows(&mut obj); // no-op
        Value::Object(obj)
    }

    #[test]
    fn wrap_npx() {
        let v = wrap(json!({"command":"npx","args":["-y","ctx7"]}));
        #[cfg(windows)]
        {
            assert_eq!(v["command"], "cmd");
            assert_eq!(v["args"], json!(["/c", "npx", "-y", "ctx7"]));
        }
        #[cfg(not(windows))]
        assert_eq!(v["command"], "npx");
    }

    #[test]
    fn wrap_already_cmd_not_doubled() {
        let v = wrap(json!({"command":"cmd","args":["/c","npx","-y","foo"]}));
        assert_eq!(v["command"], "cmd");
        assert_eq!(v["args"], json!(["/c", "npx", "-y", "foo"]));
    }

    #[test]
    fn wrap_http_type_skipped() {
        let v = wrap(json!({"type":"http","url":"https://x"}));
        assert!(!v.as_object().unwrap().contains_key("command"));
    }

    #[test]
    fn wrap_python_not_wrapped() {
        let v = wrap(json!({"command":"python","args":["server.py"]}));
        assert_eq!(v["command"], "python");
        assert_eq!(v["args"], json!(["server.py"]));
    }

    #[test]
    fn wrap_npx_cmd_suffix() {
        let v = wrap(json!({"command":"npx.cmd","args":["-y","foo"]}));
        #[cfg(windows)]
        {
            assert_eq!(v["command"], "cmd");
            assert_eq!(v["args"], json!(["/c", "npx.cmd", "-y", "foo"]));
        }
    }

    #[test]
    fn wrap_case_insensitive() {
        let v = wrap(json!({"command":"NPX","args":["-y","foo"]}));
        #[cfg(windows)]
        {
            assert_eq!(v["command"], "cmd");
            assert_eq!(v["args"], json!(["/c", "NPX", "-y", "foo"]));
        }
    }

    // ── WSL 路径检测（移植 ccs 测试）────────────────────────────────────────

    #[test]
    fn wsl_path_detection() {
        #[cfg(windows)]
        {
            assert!(is_wsl_path(Path::new(r"\\wsl$\Ubuntu\home\u\.claude.json")));
            assert!(is_wsl_path(Path::new(r"\\wsl.localhost\Debian\home\u")));
            assert!(is_wsl_path(Path::new(r"\\WSL$\Ubuntu\home\u")));
            assert!(!is_wsl_path(Path::new(r"C:\Users\u\.claude.json")));
            assert!(!is_wsl_path(Path::new(r"\\server\share\path")));
        }
        #[cfg(not(windows))]
        {
            assert!(!is_wsl_path(Path::new(r"\\wsl$\Ubuntu\home\u")));
        }
    }

    // ── 全量投影同步 ─────────────────────────────────────────────────────────

    #[test]
    fn sync_projects_enabled_and_preserves_other_keys() {
        let db = setup_db();
        let dir = unique_dir("sync-proj");
        let path = dir.join(".claude.json");
        // 预置用户已有的顶层键 + 一个未入库的 mcpServers 条目。
        std::fs::write(
            &path,
            r#"{"hasCompletedOnboarding":true,"mcpServers":{"hand":{"command":"x"}}}"#,
        )
        .unwrap();

        make_server(
            &db,
            "ctx7",
            json!({"command":"npx","args":["-y","c7"]}),
            true,
        );
        make_server(&db, "fs", json!({"command":"node"}), false); // 禁用，不投影

        sync_enabled_to_path(&db, &path).unwrap();

        let live = read_json(&path);
        assert_eq!(
            live["hasCompletedOnboarding"],
            json!(true),
            "其它顶层键保留"
        );
        let servers = live["mcpServers"].as_object().unwrap();
        assert!(servers.contains_key("ctx7"), "启用的投影");
        assert!(!servers.contains_key("fs"), "禁用的不投影");
        assert!(
            !servers.contains_key("hand"),
            "未入库的手加项被全量投影抹掉"
        );
    }

    #[test]
    fn sync_creates_file_when_missing_but_claude_dir_exists() {
        let db = setup_db();
        let dir = unique_dir("sync-create");
        let path = dir.join(".claude.json");
        // 模拟 Claude 已安装：建一个 .claude 目录兄弟（与 .claude.json 同级）。
        // should_sync_for_path 看 path.parent().join(".claude") 存在 → 同步，可建文件。
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        make_server(&db, "x", json!({"command":"node"}), true);
        sync_enabled_to_path(&db, &path).unwrap();
        assert!(path.exists(), "Claude 已安装时应写 live");
    }

    #[test]
    fn sync_skips_when_no_claude_install() {
        let db = setup_db();
        let dir = unique_dir("sync-noinstall");
        let path = dir.join(".claude.json");
        // path 不存在 + 无 .claude 兄弟目录 → should_sync=false → no-op，不建文件。
        make_server(&db, "x", json!({"command":"node"}), true);
        sync_enabled_to_path(&db, &path).unwrap();
        assert!(!path.exists(), "Claude 未安装时不应凭空建文件");
    }

    #[test]
    fn sync_root_non_object_errors_without_overwrite() {
        let db = setup_db();
        let dir = unique_dir("sync-badroot");
        let path = dir.join(".claude.json");
        std::fs::write(&path, r#"[1,2,3]"#).unwrap(); // 根非对象（path 存在 → should_sync=true）
        make_server(&db, "x", json!({"command":"node"}), true);
        let err = sync_enabled_to_path(&db, &path).unwrap_err();
        assert!(err.contains("根不是 JSON 对象"), "{}", err);
        // 不覆盖坏文件
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"[1,2,3]"#);
    }

    // ── 反向导入 ─────────────────────────────────────────────────────────────

    #[test]
    fn import_picks_up_live_servers() {
        let db = setup_db();
        let dir = unique_dir("import");
        let path = dir.join(".claude.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"ctx7":{"command":"npx","args":["-y","c7"]},"bad":{"type":"grpc"}}}"#,
        )
        .unwrap();

        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 1, "只导入合法的");
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].id, "bad");

        let row = mcp_servers::get(&db, "ctx7").unwrap().unwrap();
        assert!(row.enabled_claude, "导入的默认启用");
        assert_eq!(row.name, "ctx7", "新建用 id 作 name");
    }

    #[test]
    fn import_existing_only_enables_not_overwrite_meta() {
        let db = setup_db();
        let dir = unique_dir("import-exist");
        let path = dir.join(".claude.json");
        std::fs::write(&path, r#"{"mcpServers":{"ctx7":{"command":"newcmd"}}}"#).unwrap();

        // 先建一个已禁用、有富信息的
        mcp_servers::create(
            &db,
            NewMcpServer {
                id: "ctx7".to_string(),
                name: "我的 ctx7".to_string(),
                server_config: r#"{"command":"oldcmd"}"#.to_string(),
                description: Some("keep me".to_string()),
                homepage: Some("https://x".to_string()),
                docs: None,
                tags: r#"["a"]"#.to_string(),
                enabled_claude: false,
            },
        )
        .unwrap();

        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 1);
        let row = mcp_servers::get(&db, "ctx7").unwrap().unwrap();
        assert!(row.enabled_claude, "导入后启用");
        assert_eq!(
            row.server_config, r#"{"command":"newcmd"}"#,
            "已存在时用 live 规范更新"
        );
        assert_eq!(row.name, "我的 ctx7", "name 不覆盖");
        assert_eq!(row.description.as_deref(), Some("keep me"), "富信息不覆盖");
    }

    #[test]
    fn import_missing_file_is_noop() {
        let db = setup_db();
        let dir = unique_dir("import-missing");
        let path = dir.join(".claude.json");
        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 0);
    }

    #[test]
    fn import_root_without_mcp_servers_is_noop() {
        let db = setup_db();
        let dir = unique_dir("import-nomcp");
        let path = dir.join(".claude.json");
        std::fs::write(&path, r#"{"hasCompletedOnboarding":true}"#).unwrap();
        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 0);
    }
}
