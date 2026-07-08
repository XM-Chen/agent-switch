# Prompts 管理（CLAUDE.md 单激活 + 回填保护应用）

## Goal

P0批子任务#2：为 agent-switch 补齐 ccs 式 Claude Code **Prompts 管理**——在 SSOT（DB）里维护多份 `CLAUDE.md` 提示词，任一时刻至多一份「激活」并投影到 `~/.claude/CLAUDE.md`；激活/切换时用**回填保护**捕获 live 手改，避免用户直接改 `CLAUDE.md` 的内容被覆盖丢失。父任务 `07-07-claude-code-ccs-align` 的 P0 批，依赖已完成的 `cc-switch-semantics`（地基）。参考 ccs 实现，按 agent-switch 既有架构适配，不照抄代码。

## Background

### ccs 现状（参考蓝本，已读源码 `services/prompt.rs` + `prompt_files.rs`）
- **单激活模型**：每个 app 维护一组 prompt（DB `IndexMap<id, Prompt>`），`enable_prompt` 先把所有 prompt 置 `enabled=false`，再把目标置 `true`，并把其 content **原子写入** `prompt_file_path`（Claude = `~/.claude/CLAUDE.md`）。
- **回填保护（enable 时）**：写入目标前先读 live `CLAUDE.md`：
  - 若当前有已启用 prompt → 把 live 内容回填进「当前已启用」那份（`content = live_content` + 更新时间戳）再切换。
  - 若无已启用 prompt 且 live 内容未在任何 prompt 中出现 → 建一份 `backup-<ts>`（`enabled=false`）保存 live 原文，避免丢失。
- **disable**：目标置 `false`；若已无任何启用项 → 清空 `CLAUDE.md`（写空串）。
- **delete**：拒绝删除 `enabled=true` 的 prompt。
- **import_from_file**：把 live `CLAUDE.md` 原文导入为一份新 prompt（`enabled=false`）。
- **首次启动自动导入**：DB 该 app 无 prompt 且 live 存在非空 `CLAUDE.md` → 导入为一份 `enabled=true`（幂等：已有则跳过）。
- Prompt 字段：`id / name / content / description / enabled / created_at / updated_at`。ccs 支持多 app（Claude/Codex/Gemini/...）走 `prompt_file_path` 分派文件名（CLAUDE.md / AGENTS.md / GEMINI.md）。

### agent-switch 现状
- **无任何 Prompts 代码**：不读写 `~/.claude/CLAUDE.md`（地基任务 B1 边界已确认 grep 无 `CLAUDE.md` 写入）。
- 已确立的**外围管理范式**（刚完成的 `cc-mcp` 子任务，B1 独立解耦）：独立 service（与 `tool_takeover` 解耦，切换 provider 不触发外围 sync）+ 独立 DB 表（迁移追加）+ 标准 DAO + `http/api/*.rs` REST 路由 + 独立前端 page + 导航项。cc-prompts 完全对齐此范式。
- `atomic_write`（`services/tool_takeover/mod.rs`，已 `pub`）可复用做原子写。
- 迁移当前到 v9（`mcp_servers` 表）；本任务追加 v10。

### 关键差距
agent-switch 用户改 Claude 提示词只能手动编辑 `~/.claude/CLAUDE.md`，无版本化/多份切换/回填保护。本任务补齐 ccs 的单激活 + 回填模型。

## Requirements

> 范式锚定 cc-mcp（独立 service，与切换解耦）。「仅 Claude Code」——只投影 `~/.claude/CLAUDE.md`，不做多 app 分派（Codex 的 AGENTS.md 等留待未来）。

### R1. 数据模型：`prompts` 表（迁移 v10）
- R1.1 新增 `prompts` 表：`id / name / content / description / enabled_claude(INTEGER) / created_at / updated_at`。命名保留 `_claude` 后缀，为未来多 app 留扩展位（对齐 cc-mcp `enabled_claude` 做法）。
- R1.2 `content` 明文存储（`CLAUDE.md` 本就是明文文件，与 cc-mcp 的 `server_config` 明文权衡一致；不属 agent-switch 凭证加密边界）。
- R1.3 迁移幂等（`CREATE TABLE IF NOT EXISTS`），纯建表不回填。

### R2. DAO：`db/dao/prompts.rs`（标准 CRUD，对齐 cc-mcp 风格）
- `list / get / create / update / delete`；`get_enabled_claude`（单激活查询）；`set_enabled_exclusive`（置目标 true、其余 false，同事务）。

### R3. 单激活 + 投影
- R3.1 至多一份 prompt `enabled_claude=true`；启用目标时其余自动置 false（原子）。
- R3.2 启用 → 把该 prompt `content` 原子写入 `~/.claude/CLAUDE.md`（`should_sync` 门槛：`~/.claude` 目录或文件存在才写，对齐 cc-mcp 未安装跳过语义）。
- R3.3 禁用当前激活项且无其它激活项 → 清空 `CLAUDE.md`（写空串，对齐 ccs）。
- R3.4 删除拒绝 `enabled_claude=true` 的 prompt（对齐 ccs）。

### R4. 回填保护（enable 时捕获 live 手改）
- R4.1 启用目标前读 live `CLAUDE.md`：若存在**已启用** prompt 且 live 非空 → 把 live 内容回填进「当前已启用」那份再切换。
- R4.2 若**无**已启用 prompt 且 live 内容未在任何 prompt 中出现 → 建一份 `backup-<ts>`（`enabled=false`）保存 live 原文。
- R4.3 回填后再执行 R3.2 的投影写入。

### R5. 反向导入 + 首次导入
- R5.1 `import_from_claude`：把 live `CLAUDE.md` 原文导入为一份新 prompt（`enabled=false`，name 带时间戳），供用户纳管既有手写提示词。
- R5.2 首次启动自动导入（**纳入本任务，2026-07-07 用户拍板**）：DB 该 app 无 prompt 且 live `CLAUDE.md` 非空 → 自动导入为一份 `enabled_claude=true`（幂等：DB 已有 prompt 则跳过，对齐 ccs `import_from_file_on_first_launch`）。挂载点在应用启动初始化序列（design 阶段确认，候选：app 启动时 DB 就绪后一次性调用）。

### R6. HTTP API：`http/api/prompts.rs`（对齐 cc-mcp REST 风格）
- `GET /api/prompts`（list）/ `POST`（create）/ `GET|PUT|DELETE /{id}` / `POST /{id}/enable`（单激活+回填+投影）/ `POST /{id}/disable` / `POST /import`（反向导入）/ `GET /status`（`{ config_path, config_exists, active_prompt_id }`）。
- 写操作后按语义投影 live；校验失败 400、IO/写入失败 500。

### R7. 前端：`src/pages/PromptsPage.tsx` + `src/lib/api.ts`
- 列表（name / 激活开关 / content 摘要 / description）+ 新建/编辑弹窗（content 文本域 + name/description）+ 删除（激活项禁删）+「从 ~/.claude/CLAUDE.md 导入」+ status 展示。
- `App.tsx` 加 `/prompts` 路由；`AppShell.tsx` 导航加「Prompts」项。复用既有 `components/ui` 与 cc-mcp/providers 弹窗范式。

### R8. 解耦边界（约束）
- R8.1 独立 service，**不**并入 `tool_takeover`；切换 provider **不**触发 prompt sync（对齐 B1 + cc-mcp）。Prompt 增删改/启停即时投影。
- R8.2 不触碰 `settings.json` / `~/.claude.json`（mcpServers）/ `skills/` / `projects/`；仅读写 `~/.claude/CLAUDE.md`。
- R8.3 固定路径 `~/.claude/CLAUDE.md = home_dir().join(".claude").join("CLAUDE.md")`（不做 override_dir，与 cc-mcp 一致）。

## Acceptance Criteria

- [ ] AC1（单激活投影）：新建两份 prompt A/B，启用 A → live `CLAUDE.md` == A.content；再启用 B → live == B.content 且 A.enabled_claude=false。（R3.1–R3.2）
- [ ] AC2（回填往返）：启用 A 后在 live `CLAUDE.md` 手改内容 → 启用 B → A 的 content 已更新为手改内容（回填捕获）。（R4.1）
- [ ] AC3（无激活项备份）：DB 无激活 prompt 且 live 有非空内容 → 启用某 prompt 前，live 原文被存为 `backup-*`（enabled=false）。（R4.2）
- [ ] AC4（禁用清空）：禁用唯一激活项 → live `CLAUDE.md` 被清空。（R3.3）
- [ ] AC5（删除保护）：删除 enabled_claude=true 的 prompt 返回错误，不删。（R3.4）
- [ ] AC6（反向导入）：live 有手写 `CLAUDE.md` → import → DB 新增一份 enabled=false 且 content == live 原文。（R5.1）
- [ ] AC7（未安装跳过）：`~/.claude` 不存在时启用/投影不凭空建文件/目录（should_sync 门槛）。（R3.2）
- [ ] AC8（首次自动导入）：DB 无 prompt 且 live 有非空 `CLAUDE.md` → 启动初始化后 DB 新增一份 `enabled_claude=true` 且 content == live 原文；DB 已有 prompt 时启动不重复导入（幂等）。（R5.2）
- [ ] AC9（解耦边界）：grep 确认本任务不新增对 `settings.json`/`~/.claude.json`/`skills/`/`projects/` 的写入；切换 provider 不触发 prompt sync。（R8）
- [ ] AC10（回归）：`cargo test` + `npm test` 全绿；新增单测覆盖单激活/回填两分支/禁用清空/删除保护/导入/首次自动导入幂等/未安装跳过。

## Constraints

- 参考 ccs `services/prompt.rs` 单激活+回填思路，但按 agent-switch 架构（Rust + axum REST + SQLite + React）适配，锚定 cc-mcp 已落地范式，不照抄 ccs Tauri command 代码。
- 沿用 `atomic_write` 保证原子性；`should_sync` 门槛对齐 cc-mcp（Claude 未安装不建文件）。
- 仅 Claude Code（`CLAUDE.md`），不做多 app 分派。

## Out of Scope

- Codex `AGENTS.md` / Gemini `GEMINI.md` 等多 app 提示词（未来横切任务）。
- MCP / env 开关 / Skills / 会话 / DeepLink（其它子任务）。
- 提示词内容的模板变量/片段拼接/多段激活（ccs 也只单激活，全文替换）。

## Open Questions

（无——单激活+回填模型已由 ccs `services/prompt.rs` 精确定型，首次自动导入已拍板纳入。首次导入挂载点属技术设计，留 design.md 定。）
