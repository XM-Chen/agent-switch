# Design: MCP 统一管理（cc-mcp，仅 Claude Code）

## 范围与边界

- **仅 Claude Code**：只读写 `~/.claude.json` 的 `mcpServers` 字段。不碰 codex/gemini/opencode（本任务不引入 per-app 开关矩阵，只有单一「启用」布尔）。
- **独立 service**：MCP 管理与 `tool_takeover` 完全解耦（对齐 B1 决策）。切换 provider 不触发 MCP 同步；MCP 增删改即时同步。
- **固定路径**：`~/.claude.json` = `home_dir().join(".claude.json")`。不做 override_dir（归后续横切任务）。
- **不进 takeover 快照**：MCP 不属于 `settings.json`，与地基任务的 `meta.snapshot` / common config 层无交集。

## 数据模型

新增 `mcp_servers` 表（迁移 v9，`db/migrations.rs` 末尾追加）：

```sql
CREATE TABLE IF NOT EXISTS mcp_servers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    server_config TEXT NOT NULL,      -- JSON：纯 MCP 规范（command/args/env 或 type/url/headers）
    description  TEXT,
    homepage     TEXT,
    docs         TEXT,
    tags         TEXT NOT NULL DEFAULT '[]',  -- JSON 数组
    enabled_claude INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
```

- `server_config`：只存纯规范（写 live 时原样投影，不再剥离）。富信息（description/homepage/docs/tags）单列存放，仅供 UI 展示，**不写入 live**。
- `enabled_claude`：单一启用开关（本任务只有 Claude Code 一个目标）。命名保留 `_claude` 后缀，为将来多应用留扩展位（对齐 ccs 列名，减少未来迁移）。
- **加密边界**：MCP server 规范可能含 env 里的 token（如 API key）。但 MCP 规范本就是明文写进 `~/.claude.json` 的（Claude Code 读明文），DB 存明文与 live 一致，**不引入加密**（与 ccs 一致；这不是 agent-switch 的凭证加密边界——那只覆盖 endpoints/accounts 的 direct 凭证）。PRD 需注明此权衡。

## DAO：`db/dao/mcp_servers.rs`

标准 CRUD（对齐 `providers.rs` DAO 风格）：

- `list(db) -> Vec<McpServerRow>`（按 name 或 created_at 排序）
- `get(db, id) -> Option<McpServerRow>`
- `create(db, NewMcpServer) -> McpServerRow`
- `update(db, id, McpServerUpdate)`（部分字段，嵌套 Option 语义对齐 providers）
- `delete(db, id)`
- `list_enabled_claude(db) -> Vec<McpServerRow>`（同步投影用）
- `upsert(db, id, ...)`（反向导入用；存在则更新规范+启用，不存在则建）

`McpServerRow`：id/name/server_config(String)/description/homepage/docs/tags(String)/enabled_claude(bool)/created_at/updated_at。

## Service：`services/mcp/mod.rs` + `services/mcp/claude.rs`

移植 ccs `mcp/claude.rs` + `claude_mcp.rs`，按 agent-switch 风格改写（`Result<_, String>`，复用既有 `atomic_write`）。

### 路径解析

```rust
fn claude_mcp_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".claude.json"))
        .ok_or_else(|| "无法获取用户主目录".to_string())
}
```

### 全量投影写入（核心）

`sync_enabled_to_claude(db)`：
1. `should_sync()`：`~/.claude` 目录或 `~/.claude.json` 存在才同步（对齐 ccs：Claude 未安装时跳过，不凭空建文件）。
2. 读 `~/.claude.json` 全文（不存在 → `{}`）。
3. `list_enabled_claude` → 组装 `mcpServers` map：每个 server 取 `server_config` 解析为 JSON + Windows `cmd /c` 包装（非 WSL 路径）。
4. **只替换根对象的 `mcpServers` 字段**，保留其它顶层键（用户手加的 `hasCompletedOnboarding` 等）。
5. `atomic_write`（`.tmp` → rename，pretty JSON）。

> 全量投影：`mcpServers` 字段整体由 DB enabled 集合决定。用户手加进 `~/.claude.json` 但不在 DB 的 server 会被抹掉 → 由「反向导入」缓解（见下）。其它顶层键不受影响。

### 反向导入

`import_from_claude(db) -> Result<ImportReport, String>`：
1. 读 `~/.claude.json` 的 `mcpServers`。
2. 逐项校验（`validate_server_spec`）；无效项跳过并计入 report，不中止。
3. `upsert`：DB 已有同 id → 置 `enabled_claude=true`（不覆盖富信息）；无 → 新建（`enabled_claude=true`，name=id）。
4. 返回 `{ imported, skipped: Vec<{id, reason}> }`。

### Windows cmd /c 包装 + WSL 检测

逐字移植 ccs `wrap_command_for_windows`（`WINDOWS_WRAP_COMMANDS = [npx,npm,yarn,pnpm,node,bun,deno]`，`#[cfg(windows)]`）+ `is_wsl_path`（`\\wsl$\` / `\\wsl.localhost\` UNC 前缀检测）。仅 stdio 类型包装；已是 cmd 的不重复包装。非 Windows 为 no-op。

### 校验

移植 `validate_server_spec`：object 必需；`type` ∈ {stdio(缺省), http, sse}；stdio 需 `command`，http/sse 需 `url`。

## HTTP API：`http/api/mcp.rs`

```
GET    /api/mcp                → list（含富信息，供 UI）
POST   /api/mcp                → create（校验规范）→ 同步 live
GET    /api/mcp/{id}           → get
PUT    /api/mcp/{id}           → update（含 enabled_claude 切换）→ 同步 live
DELETE /api/mcp/{id}           → delete → 同步 live
POST   /api/mcp/sync           → 手动全量同步（幂等，兜底）
POST   /api/mcp/import         → 反向导入 ~/.claude.json → 同步 live（导入后 enabled 项落 live）
GET    /api/mcp/status         → { config_path, config_exists, live_server_count }
```

- 任何改动 DB 的写操作（create/update/delete/import）成功后调用 `sync_enabled_to_claude`，保证 DB↔live 一致。
- 校验失败 → 400；`~/.claude.json` 根非对象 → 400（不破坏用户文件）；IO/写入失败 → 500。
- 在 `http/router.rs` `.nest("/api/mcp", api::mcp::routes())`；`http/api/mod.rs` `pub mod mcp;`。

## 前端

- `src/lib/api.ts`：`mcpApi { list, create, get, update, remove, sync, import, status }` + `McpServer` / `CreateMcpBody` / `UpdateMcpBody` / `McpImportReport` 类型。
- `src/pages/McpPage.tsx`：列表（name/启用开关/规范摘要/tags）+ 新建/编辑弹窗（裸 JSON 规范编辑器 + 富信息字段）+ 删除 + 「从 ~/.claude.json 导入」按钮 + status 展示。
- `App.tsx` 加 `/mcp` 路由；`AppShell.tsx` 导航加「MCP」项（icon 🧩）。
- 复用既有 `components/ui` 与 providers 页的弹窗/表单模式。

## 兼容 / 回归

- **无既有 MCP 代码** → 纯新增，不改地基任务的 tool_takeover / snapshot / common config 路径。
- 迁移 v9 只 `CREATE TABLE`，幂等（`IF NOT EXISTS`），不回填数据。
- 不触碰 `settings.json`、代理网关、ccs 导入器、portability。

## 风险 / 回滚点

- **全量投影抹掉手加 server**：最高风险。缓解 = 反向导入 + 保留其它顶层键 + 首次同步前建议用户先导入。文档需提示。
- `~/.claude.json` 根非对象（用户写坏）：读到后 400 报错，不覆盖。
- Windows 包装遗漏/误包：单测覆盖 npx/http/已 cmd/非目标命令/WSL 各分支（移植 ccs 测试）。
- 回滚：删表 + 摘掉路由即可，无数据迁移副作用。
