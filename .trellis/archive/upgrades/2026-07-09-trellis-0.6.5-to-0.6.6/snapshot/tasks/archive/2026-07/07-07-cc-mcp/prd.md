# MCP 统一管理（~/.claude.json mcpServers 字段级增量）

## Goal

为 agent-switch 补齐 ccs 式的 Claude Code MCP 服务器管理：用户在 agent-switch UI 里 CRUD 一份全局 MCP 服务器清单，启用的服务器即时投影写入 `~/.claude.json` 的 `mcpServers` 字段。填平 agent-switch 当前「完全不管理 MCP」的功能缺口（父任务 `07-07-claude-code-ccs-align` 子任务 3）。

## Background

### agent-switch 现状
- **完全不管理 MCP**：确认（读 `tool_takeover/mod.rs` + `claude_code.rs` 全文 + grep）agent-switch 不碰 `~/.claude.json`，无 MCP service、无 `mcp_servers` 表。切换只接管 `~/.claude/settings.json` 的连接层 env（地基任务 `cc-switch-semantics` 已实现快照层）。
- 现有配置文件写入模式：`tool_takeover::atomic_write`（`.tmp`→rename）。DB 迁移走 `db/migrations.rs` 的 `MIGRATIONS` 数组（末尾追加，当前最高 version=8）。kv 设置走 `app_metadata` 表。
- 前端：React + react-router，页面在 `src/pages/`，侧边栏导航在 `components/layout/AppShell.tsx`，API 客户端在 `src/lib/api.ts`。

### ccs 现状（参考来源，本地 `E:/SynologyDrive/git_files/cc-switch/cc-switch-main`）
- `mcp_servers` 表（`database/schema.rs:64`）：`id, name, server_config, description, homepage, docs, tags, enabled_claude, enabled_codex, enabled_gemini, enabled_opencode, enabled_hermes`。跨多应用 per-app 启用开关。
- 写 live（`mcp/claude.rs` + `claude_mcp.rs`）：**全量投影** `set_mcp_servers_map` —— 读整个 `~/.claude.json` → 只替换 `mcpServers` 字段（保留其它顶层键）→ 原子写。写回前剥离 UI 辅助字段（`enabled/source/id/name/description/tags/homepage/docs`），只留实际规范。
- Windows `cmd /c` 包装（`claude_mcp.rs:wrap_command_for_windows`）：stdio 类型且 command 属于 `npx/npm/yarn/pnpm/node/bun/deno` 时，改写为 `command="cmd"`, `args=["/c", 原command, ...原args]`。已是 cmd 的不重复包装。
- WSL 路径检测（`is_wsl_path`）：`~/.claude.json` 落在 `\\wsl$\...` / `\\wsl.localhost\...` UNC 路径时跳过 cmd /c 包装（WSL 跑 Linux 不需要）。仅 Windows 生效。
- 校验（`mcp/validation.rs`）：spec 须为对象；type ∈ {stdio,http,sse}（缺省按 stdio）；stdio 需非空 `command`；http/sse 需非空 `url`。
- 反向导入（`mcp/claude.rs:import_from_claude`）：从 live `~/.claude.json` 的 mcpServers 读入 DB，已存在的只启用 claude，不存在的新建。单项校验失败不中止（收集错误继续）。
- `~/.claude.json` 路径：`config.rs:get_default_claude_mcp_path` = `home.join(".claude.json")`（是 `~/.claude/` 目录的**兄弟文件**，不在 config_dir 内）。

## Requirements

按已拍板决策，范围锁定 **仅 Claude Code、独立全局清单、全量投影、固定路径**。

### R1. `mcp_servers` 表（仅 Claude Code 启用开关）
- R1.1 新增 DB 迁移（`MIGRATIONS` 末尾追加 version=9）：`mcp_servers` 表。列：`id TEXT PK`、`name TEXT`、`server_config TEXT`（JSON 规范：command/args/env/type/url 等）、`description TEXT`、`homepage TEXT`、`docs TEXT`、`tags TEXT NOT NULL DEFAULT '[]'`、`enabled_claude INTEGER NOT NULL DEFAULT 0`、`created_at TEXT`、`updated_at TEXT`。
- R1.2 只做 Claude Code 一个启用开关（`enabled_claude`），不引入 codex/gemini/opencode/hermes 列（范围决策：仅 Claude Code）。
- R1.3 迁移幂等（`CREATE TABLE IF NOT EXISTS`），与既有迁移风格一致（DDL only，不在迁移 SQL 里做数据回填）。

### R2. MCP DAO + Service（CRUD + 全量投影同步）
- R2.1 新增 `db/dao/mcp_servers.rs`：`create` / `get` / `list` / `update` / `delete` / `list_enabled_claude`。server_config 按原样存 JSON 文本（不加密，MCP 规范无凭据加密需求；若 server_config 含敏感 env 由用户自负，与 ccs 一致）。
- R2.2 新增 MCP service（落点 `services/tool_takeover/claude_mcp.rs` 或 `services/mcp.rs`，design 定）：`sync_enabled_to_live(db, config_dir_or_path)` —— 取所有 `enabled_claude=1` 的 server → 剥离富信息列只留 server_config 规范 → Windows cmd /c 包装（WSL 跳过）→ **全量投影**写入 `~/.claude.json` 的 `mcpServers` 字段（保留其它顶层键）→ 原子写。
- R2.3 校验（移植 ccs `validation.rs`）：spec 须为对象；type ∈ {stdio,http,sse}（缺省 stdio）；stdio 需 command；http/sse 需 url。create/update 时校验，非法拒绝。
- R2.4 即时同步：任何 CRUD 操作（含启用开关切换）成功写 DB 后，立即调用 R2.2 的全量投影，使 live 与 DB 一致。MCP 同步**独立于 provider 切换**，不挂在 `perform_switch` 上。

### R3. Windows cmd /c 包装 + WSL 检测（移植 ccs）
- R3.1 移植 `wrap_command_for_windows`：`#[cfg(windows)]` 下，stdio 类型且 command ∈ {npx,npm,yarn,pnpm,node,bun,deno}（大小写不敏感、忽略 .cmd 后缀）→ 改写为 cmd /c 包装。已是 cmd 的不重复包装。非 Windows 为 no-op。
- R3.2 移植 `is_wsl_path`：`#[cfg(windows)]` 下检测 `\\wsl$\` / `\\wsl.localhost\` UNC 前缀（大小写不敏感）。目标路径为 WSL 时跳过 cmd /c 包装。
- R3.3 单测覆盖 R3.1/R3.2 全部分支（移植 ccs 现有测试用例）。

### R4. 反向导入（一次性按钮，缓解全量投影抹改风险）
- R4.1 新增 `import_from_live(db, path)`：读 live `~/.claude.json` 的 mcpServers → 逐条校验（失败跳过并收集错误，不中止）→ DB 已存在同 id 的只置 `enabled_claude=1`（不覆盖 server_config）、不存在的新建（默认 `enabled_claude=1`）。返回导入计数 + 错误列表。
- R4.2 HTTP API 暴露为一次性触发（`POST /api/mcp/import-from-live`），前端提供「从 live 导入」按钮。
- R4.3 反向导入不自动触发（避免与全量投影循环），仅用户显式点击。

### R5. HTTP API + 前端完整 CRUD UI
- R5.1 HTTP API（新模块 `http/api/mcp.rs`，router nest `/api/mcp`）：`GET /api/mcp`（列表）、`POST /api/mcp`（创建）、`PUT /api/mcp/{id}`（更新，含 enabled_claude 切换）、`DELETE /api/mcp/{id}`、`POST /api/mcp/import-from-live`。
- R5.2 前端新增 MCP 页面（`src/pages/McpPage.tsx` + 侧边栏导航项 + 路由）：列表展示、新增/编辑（裸 JSON 编辑 server_config + name/description 等）、删除、enabled_claude 开关、从 live 导入按钮。
- R5.3 `src/lib/api.ts` 加 `mcpApi` 绑定。

### R6. 边界约束
- R6.1 `~/.claude.json` 路径用**固定** `home.join(".claude.json")`（不引入 ccs 的 override_dir；override 是横切能力，归独立任务统一给 settings/.claude.json/CLAUDE.md/skills）。
- R6.2 只写 `~/.claude.json` 的 `mcpServers` 字段，保留其它顶层键（`hasCompletedOnboarding` 等用户数据不动）。
- R6.3 不碰 `~/.claude/settings.json`（地基任务范围）、`CLAUDE.md`（cc-prompts）、`skills/`（cc-skills）。
- R6.4 `~/.claude.json` 不存在且 `~/.claude/` 目录不存在时（Claude 未安装）→ 同步跳过，不创建文件（对齐 ccs `should_sync_claude_mcp`）。

## Acceptance Criteria

- [ ] AC1（表 + CRUD）：迁移后 `mcp_servers` 表存在；经 API 创建/编辑/删除 MCP server，DB 行正确；迁移可重复执行不损坏数据。（R1/R2.1）
- [ ] AC2（全量投影同步）：启用 2 个 server → live `~/.claude.json` 的 `mcpServers` 恰含这 2 个（规范字段，无 UI 辅助字段）；禁用 1 个 → live 只剩 1 个；`~/.claude.json` 其它顶层键（如 `hasCompletedOnboarding`）保持不变；文件原子写。（R2.2/R2.4/R6.2）
- [ ] AC3（校验）：type=stdio 缺 command → 拒绝；type=http 缺 url → 拒绝；type 非法 → 拒绝；缺省 type 按 stdio 处理。（R2.3）
- [ ] AC4（Windows 包装）：`#[cfg(windows)]` 下 stdio + command=npx → 写 live 为 `command=cmd, args=["/c","npx",...]`；command=python 不包装；已是 cmd 不重复包装；http 类型不包装。非 Windows 为 no-op。（R3.1，单测）
- [ ] AC5（WSL 检测）：`#[cfg(windows)]` 下 `\\wsl$\Ubuntu\...` / `\\wsl.localhost\...` 判定为 WSL（大小写不敏感）；`C:\...` / 普通 UNC 判定为非 WSL。（R3.2，单测）
- [ ] AC6（反向导入）：live `~/.claude.json` 手加 server X（不在 DB）→ 点「从 live 导入」→ DB 含 X 且 `enabled_claude=1`；已存在同 id 的只置启用不覆盖规范；非法条目跳过并报错计数。（R4）
- [ ] AC7（未安装跳过）：`~/.claude.json` 与 `~/.claude/` 均不存在时，同步为 no-op 不建文件。（R6.4）
- [ ] AC8（前端 CRUD）：MCP 页面可列出/新增/编辑/删除/切换启用/从 live 导入；`npm run build && npm test` 通过。（R5）
- [ ] AC9（边界 + 回归）：grep 确认本任务不写 `settings.json`/`CLAUDE.md`/`skills/`；`cargo test` + 现有 tool_takeover/切换测试全绿；新增单测覆盖投影/校验/包装/WSL/导入。（R6/回归）

## Out of Scope

- codex/gemini/opencode/hermes 的 MCP 管理（本任务仅 Claude Code；多应用统一为未来扩展）。
- `~/.claude.json` 的 override_dir / 自定义目录（横切任务统一实现）。
- MCP server 凭据加密（MCP 规范的 env 由用户自负，与 ccs 一致；agent-switch 的加密边界针对 provider/endpoint 凭据）。
- CLAUDE.md（cc-prompts）、skills（cc-skills）、env 行为开关（cc-env-switches）、settings.json 快照层（cc-switch-semantics 已完成）。
- Deep Link 导入 MCP（cc-deeplink）。

## Decisions（已拍板 2026-07-07）

- **范围 = 仅 Claude Code**：单个 `enabled_claude` 开关，不做多应用列。
- **触发 = 独立全局清单 + 即时同步**：MCP 是全局清单，CRUD 后即时投影写 live，不绑定 provider 切换。
- **写入语义 = 全量投影**：DB 里 enabled 的整体投影覆盖 `~/.claude.json` 的 mcpServers 字段（保留其它顶层键）。
- **反向导入 = 一次性按钮**：提供显式「从 live 导入」缓解全量投影抹掉用户手改的风险，不自动触发。
- **前端 = 完整 CRUD UI**：MCP 没有独立编辑器子任务，UI 落在本任务。
- **路径 = 固定 `home.join(".claude.json")`**：不做 override_dir（归横切任务，避免与 settings.json 接管路径分叉）。
- **server_config 列 = 规范 + 富信息列**：对齐 ccs 表结构存 description/homepage/docs/tags，写 live 时剥离只留规范。
