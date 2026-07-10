# Skills 管理（完整 ccs 范围）

## Goal

P1 子任务：为 agent-switch 补齐 ccs 式 Skills 管理，覆盖 Skills SSOT、`~/.claude/skills` 投影、多应用复用、GitHub/skills.sh 发现、导入/备份/恢复与批量更新。用户已选择按“完整 ccs”范围规划，而非仅 Claude Code MVP。

## Background

### 已确认的 agent-switch 现状

- 当前已落地 `cc-mcp` / `cc-prompts`，但没有 Skills 模块：DB 迁移已有 `mcp_servers` 与 `prompts`，未见 `skills` 表。
- 当前后端路由已挂 `/api/mcp`、`/api/prompts`、`/api/common-config`，没有 `/api/skills`。
- 当前前端路由和侧栏已有 `/mcp`、`/prompts`，没有 Skills 页面。
- 可复用的持久化根目录是应用数据目录（`app_data_dir()`），适合作为 agent-switch 自有 Skills SSOT。
- 当前工具接管路径只覆盖 Claude Code 与 Codex；Skills 完整 ccs 范围会引入未来/外部 app 目标（Gemini/OpenCode 等）的路径投影能力，需与 agent-switch 现有 app 支持边界清晰隔离。
- 已有文件写入工具包括写前备份与 `.tmp -> rename` 原子写；尚无通用目录 symlink/copy 同步工具。
- B1 外围管理范式已经收敛为独立 service：CRUD 后即时投影，不绑定 provider 切换。`cc-skills` 应延续该范式。

### ccs 参考事实

- ccs Skills 表是多应用结构：`enabled_claude/codex/gemini/opencode/hermes`，另有 `skill_repos` 表。
- ccs Skills SSOT 默认 `~/.cc-switch/skills/`，也支持统一目录 `~/.agents/skills/`。
- ccs 应用投影目录包括 `~/.claude/skills`、`~/.codex/skills`、`~/.gemini/skills`、OpenCode config skills 等。
- ccs 同步方式支持 `auto/symlink/copy`：auto 优先 symlink，失败回退 copy；copy 用临时目录再 rename。
- ccs 触发点包括安装后同步、toggle 后同步/移除、全量 `sync_to_app` 清理并重建启用项、导入/zip/更新后同步。
- ccs UI/API 覆盖 installed/backups/install/uninstall/toggle/scan/import/discover/update/migrate/search。

## Requirements

### R1. 数据模型：Skills 与 Skill Repos

- R1.1 新增 DB 迁移创建 `skills` 表，字段至少包括：`id`、`name`、`description`、`directory`、`source_type`、`source_url`、`repo_owner`、`repo_name`、`repo_branch`、`repo_subdir`、`readme_url`、`enabled_claude`、`enabled_codex`、`enabled_gemini`、`enabled_opencode`、`enabled_hermes`、`installed_at`、`updated_at`、`content_hash`、`created_at`。
- R1.2 新增 `skill_repos` 表或等价存储，用于记录发现/安装来源、仓库元数据、批量更新状态。
- R1.3 迁移幂等，不在迁移 SQL 中做外部文件扫描或网络访问。

### R2. SSOT 与存储位置

- R2.1 提供 agent-switch 自有 SSOT：默认位于应用数据目录下的 `skills/`。
- R2.2 支持 ccs 式统一目录 `~/.agents/skills/` 作为可选 SSOT 模式，并提供迁移/切换流程。
- R2.3 每个 skill 目录必须包含有效入口文件（如 `SKILL.md` 或 ccs 兼容约定），导入时校验目录结构。
- R2.4 SSOT 中的 skill 内容需可计算 hash，用于检测更新、冲突与备份恢复。

### R3. 多应用投影与同步

- R3.1 支持按 app 启用/禁用：Claude Code、Codex、Gemini、OpenCode、Hermes（按 ccs 完整范围建模）。
- R3.2 Claude Code 投影目标为 `~/.claude/skills/<directory>`；其它 app 的目标路径按 ccs 兼容实现，并在 agent-switch UI 中标注“外部工具路径”。
- R3.3 同步方式支持 `auto` / `symlink` / `copy`：auto 优先 symlink，失败时 fallback copy 并返回提示。
- R3.4 目标 app 配置目录不存在时，不凭空创建根目录；DB 启用状态可保存，status 显示“未投影/目标不存在”。
- R3.5 全量 sync 以 DB enabled 状态为准重建托管项，但不得删除非 agent-switch 托管目录。
- R3.6 目标已有非托管同名目录/文件时，不直接覆盖；返回 conflict，并提供扫描/导入/替换确认路径。

### R4. 导入、安装、卸载、备份与恢复

- R4.1 支持从本地目录导入 skill。
- R4.2 支持从 zip 导入 skill。
- R4.3 支持从 GitHub repo/subdir 安装 skill，并记录 repo 元数据。
- R4.4 卸载前创建备份，删除 DB、SSOT 与托管 live 投影；恢复备份后按当前 enabled 状态重新投影。
- R4.5 支持扫描未管理 live skills 并导入纳管。

### R5. 发现、搜索与批量更新

- R5.1 支持 ccs 式 GitHub/skills.sh 发现入口，至少能搜索/展示候选 skill，并从候选安装。
- R5.2 支持检查已安装 GitHub 来源 skill 的更新状态。
- R5.3 支持单个更新与批量更新；更新前备份，更新失败可回滚。
- R5.4 网络访问必须是用户显式触发，不在应用启动时自动拉取外部资源。

### R6. HTTP API 与前端 UI

- R6.1 新增 `services/skills`、`db/dao/skills.rs`、`http/api/skills.rs` 与 `/api/skills` REST 路由。
- R6.2 API 覆盖 list/get/import-dir/import-zip/install-repo/enable/disable/sync/status/scan-unmanaged/backups/restore/update/check-updates/search。
- R6.3 前端新增 `/skills` 页面与侧栏项，覆盖已安装列表、按 app 启用开关、导入/安装、同步状态、冲突处理、备份恢复、搜索发现与批量更新。
- R6.4 前端必须区分 loading/error/empty/conflict 状态；涉及删除、覆盖、网络安装、批量更新等 hard-to-reverse 或 outward-facing 操作必须二次确认。

### R7. 安全与边界

- R7.1 路径操作必须限制在 SSOT、备份目录和已知 app skills 目标目录内，不接受任意路径删除。
- R7.2 symlink/copy 同步不得跟随恶意 symlink 删除越界文件。
- R7.3 不触碰 `settings.json`、`~/.claude.json`、`CLAUDE.md`、`projects/`。
- R7.4 不绑定 provider 切换；Skills CRUD/toggle/sync 后即时投影。
- R7.5 外部 repo/skills.sh 数据视为不可信，导入前展示预览并要求确认。

## Acceptance Criteria

- [ ] AC1：迁移后 `skills` 与 `skill_repos`（或等价）存在，重复迁移安全。（R1）
- [ ] AC2：导入包含有效入口文件的本地目录/zip 后，内容进入 SSOT，DB 记录正确，hash 可计算；无效目录被拒绝。（R2/R4）
- [ ] AC3：启用 Claude skill 时，若 `~/.claude` 存在，则 `~/.claude/skills/<directory>` 以 symlink 或 fallback copy 投影；禁用后只移除托管项。（R3）
- [ ] AC4：Codex/Gemini/OpenCode/Hermes 启用开关按 ccs 目标路径投影；目标根目录不存在时不创建根目录并显示未投影状态。（R3）
- [ ] AC5：目标已有非托管同名目录时启用返回 conflict，不覆盖；扫描未管理后可导入纳管。（R3/R4）
- [ ] AC6：卸载前创建备份；restore 后恢复 SSOT/DB，并按 enabled 状态重新投影。（R4）
- [ ] AC7：GitHub repo/subdir 安装成功记录 repo 元数据；检查更新、单个更新、批量更新可用；更新失败可回滚。（R5）
- [ ] AC8：skills.sh/GitHub 发现入口可搜索并安装候选；网络访问仅由用户显式触发。（R5/R7）
- [ ] AC9：手动 sync 以 DB enabled 状态重建 live 托管项，不误删非托管目录。（R3/R7）
- [ ] AC10：前端 `/skills` 可完成列表、导入、安装、启用切换、同步、冲突处理、删除/恢复、搜索发现、批量更新。（R6）
- [ ] AC11：路径安全测试覆盖越界、恶意 symlink、非托管同名目录；不新增对 `settings.json`、`~/.claude.json`、`CLAUDE.md`、`projects/` 的写入。（R7）
- [ ] AC12：`cargo test`、`npm test`、`npm run build` 通过；新增单测覆盖 DAO、同步、导入、备份、更新与前端关键 helper。（R1-R7）

## Constraints

- 用户已选择“完整 ccs”范围；实现计划必须拆阶段控制风险，但 PRD 记录完整目标。
- 参考 ccs 行为但按 agent-switch Rust + axum REST + SQLite + React 架构适配，不照抄 Tauri command 形态。
- 外围管理独立 service，不挂 provider switch。
- 所有文档与 UI 文案使用中文。

## Out of Scope

- Claude Code provider 切换语义、MCP、Prompts、Sessions、DeepLink 的实现本体。
- 自动在应用启动时联网发现或更新 skills。
- 执行 skill 内容或校验 skill 内脚本安全性；本任务只管理文件安装/投影。

## Open Questions

- 无阻塞产品问题。已按用户选择将 Skills 规划为完整 ccs 范围；执行时可按 implement.md 阶段拆分。