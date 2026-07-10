# Design: Prompts 管理（cc-prompts，仅 Claude Code）

## 范围与边界

- **仅 Claude Code**：只读写 `~/.claude/CLAUDE.md`。不做多 app 分派（Codex `AGENTS.md` / Gemini `GEMINI.md` 留待未来横切任务）。
- **独立 service**：与 `tool_takeover` 完全解耦（B1 决策）。切换 provider **不**触发 prompt sync；prompt 启用/禁用/增删改即时投影。
- **锚定 cc-mcp 范式**：独立表（迁移 append）+ 标准 DAO + `services/prompts` service + `http/api/prompts.rs` REST + `PromptsPage` + 导航项。逐层对齐已归档的 cc-mcp 结构，降低设计与 review 成本。
- **固定路径**：`~/.claude/CLAUDE.md = home_dir().join(".claude").join("CLAUDE.md")`。不做 override_dir（与 cc-mcp 一致）。
- **不进 takeover 快照**：`CLAUDE.md` 不属于 `settings.json`，与地基 `meta.snapshot` / common config 层无交集。

## 数据模型

新增 `prompts` 表（迁移 v10，`db/migrations.rs` 末尾追加）：

```sql
CREATE TABLE IF NOT EXISTS prompts (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    content        TEXT NOT NULL,
    description    TEXT,
    enabled_claude INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
```

- `content`：明文存储（`CLAUDE.md` 本就是明文文件，与 cc-mcp `server_config` 明文权衡一致；不属 agent-switch 凭证加密边界）。
- `enabled_claude`：单激活开关，`_claude` 后缀为未来多 app 留扩展位（对齐 cc-mcp 列名）。
- 迁移只 `CREATE TABLE IF NOT EXISTS`，幂等，不回填数据。

## DAO：`db/dao/prompts.rs`

标准 CRUD（对齐 `mcp_servers.rs` DAO 风格，`Result<_, String>`）：

- `list(db) -> Vec<PromptRow>`（按 created_at 或 name 排序）
- `get(db, id) -> Option<PromptRow>`
- `create(db, NewPrompt) -> PromptRow`
- `update(db, id, PromptUpdate)`（部分字段，嵌套 Option 对齐 providers/mcp）
- `delete(db, id)`
- `get_enabled_claude(db) -> Option<PromptRow>`（单激活查询）
- `set_enabled_exclusive(db, id)`：**同一事务**内把目标置 `enabled_claude=1`、其余全部置 0。保证至多一份激活。

`PromptRow`：id/name/content/description/enabled_claude(bool)/created_at/updated_at。

## Service：`services/prompts/mod.rs` + `services/prompts/claude.rs`

移植 ccs `services/prompt.rs` 的单激活 + 回填模型，按 agent-switch 风格改写（`Result<_, String>`，复用 `tool_takeover::atomic_write`）。

### 路径解析 + should_sync（对齐 cc-mcp）

```rust
fn claude_md_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".claude").join("CLAUDE.md"))
        .ok_or_else(|| "无法获取用户主目录".to_string())
}
```

`should_sync_for_path(path)`：由 path 自身 parent 推导 `.claude` 目录是否存在（对齐 cc-mcp 的 hermetic 修正：生产语义正确 + 测试隔离，不看真实 home）。Claude 未安装（`~/.claude` 不存在）→ 跳过写入，不凭空建目录/文件。

### enable（单激活 + 回填 + 投影，核心）

`enable_prompt(db, id)`：
1. **回填保护**：读 live `CLAUDE.md`（存在且非空时）：
   - 若 DB 有已启用 prompt → 把 live 内容回填进「当前已启用」那份（`content = live`，更新 `updated_at`）。
   - 若 DB 无已启用 prompt 且 live 内容未在任何 prompt 的 content 中出现 → 建一份 `backup-<ts>`（`enabled_claude=0`）保存 live 原文（避免重复备份：内容已存在则跳过）。
2. `set_enabled_exclusive(db, id)`（目标置 1，其余置 0，同事务）。
3. 投影：`should_sync` 通过时把目标 prompt `content` **原子写入** `CLAUDE.md`。
4. 目标 id 不存在 → 错误，不改任何状态。

### disable

`disable_prompt(db, id)`：目标置 `enabled_claude=0`；若已无任何启用项且 `should_sync` 通过 → 清空 `CLAUDE.md`（写空串，对齐 ccs）。

### delete

`delete_prompt(db, id)`：目标 `enabled_claude=1` → 拒绝（错误）；否则删。

### 反向导入 + 首次自动导入

- `import_from_claude(db) -> ImportReport`：读 live `CLAUDE.md`，非空则导入为一份新 prompt（`enabled_claude=0`，name 带时间戳 + description "从现有配置导入"）。空/不存在 → no-op（report 标注）。
- `import_on_first_launch(db) -> usize`：**幂等** —— DB 已有任意 prompt → 返回 0 跳过；DB 空且 live `CLAUDE.md` 非空 → 导入为一份 `enabled_claude=1`（首次自动启用，对齐 ccs `import_from_file_on_first_launch`）。返回导入条数。

### get_status

`get_status(db) -> { config_path, config_exists, active_prompt_id }`（对齐 cc-mcp status 风格）。

## HTTP API：`http/api/prompts.rs`

```
GET    /api/prompts            → list
POST   /api/prompts            → create（不自动启用）
GET    /api/prompts/{id}       → get
PUT    /api/prompts/{id}       → update（name/content/description）
DELETE /api/prompts/{id}       → delete（激活项拒删 → 409/400）
POST   /api/prompts/{id}/enable   → 单激活 + 回填 + 投影
POST   /api/prompts/{id}/disable  → 禁用（唯一激活项则清空 live）
POST   /api/prompts/import        → 反向导入 ~/.claude/CLAUDE.md
GET    /api/prompts/status        → { config_path, config_exists, active_prompt_id }
```

- 写操作按语义投影 live；校验失败 400、删除激活项 400、IO/写入失败 500（对齐 cc-mcp `map_sync_error` 风格）。
- `http/api/mod.rs` `pub mod prompts;`；`http/router.rs` `.nest("/api/prompts", api::prompts::routes())`。

## 首次自动导入挂载点

在应用启动初始化序列、DB 迁移完成后，一次性调用 `prompts::import_on_first_launch(db)`（幂等保护自身兜底）。挂载位置对齐既有启动初始化（如 `app_state` 构建后 / main setup），design 落地时确认具体函数；失败仅告警不阻断启动（与 ccs `import_from_file_on_first_launch` 的 `Ok(0)` 容错一致）。

## 前端

- `src/lib/api.ts`：`promptsApi { list, create, get, update, remove, enable, disable, import, status }` + `Prompt` / `CreatePromptBody` / `UpdatePromptBody` / `PromptImportReport` / `PromptStatus` 类型。
- `src/pages/PromptsPage.tsx`：列表（name / 激活开关 / content 摘要 / description）+ 新建/编辑弹窗（content 文本域 + name/description）+ 删除（激活项禁删）+「从 CLAUDE.md 导入」按钮 + status 展示。
- `App.tsx` 加 `/prompts` 路由；`AppShell.tsx` 导航加「Prompts」项。复用既有 `components/ui` 与 cc-mcp/providers 弹窗范式。

## 兼容 / 回归

- **无既有 Prompts 代码** → 纯新增，不改地基 tool_takeover / snapshot / common config，也不改 cc-mcp。
- 迁移 v10 只 `CREATE TABLE`（幂等），不回填。
- 不触碰 `settings.json`、`~/.claude.json`、`skills/`、`projects/`、代理网关、ccs 导入器、portability。

## 风险 / 回滚点

- **回填两分支易错**：有已启用项 → 回填进该项；无启用项 → 备份。单测须覆盖两分支 + 重复备份跳过。
- **单激活事务性**：`set_enabled_exclusive` 必须同事务，避免出现 0 或 2 份激活。单测覆盖切换 A→B 后 A.enabled=0。
- **首次导入幂等**：DB 非空必须跳过；单测覆盖「已有 prompt → 启动不重复导入」。
- **未安装跳过**：`~/.claude` 不存在时 enable/disable 投影不建目录/文件；单测覆盖。
- **清空语义**：禁用唯一激活项写空串（非删文件），对齐 ccs。
- 回滚：删表 + 摘路由 + 撤首次导入调用即可，无数据迁移副作用。
