# 实现计划：MCP 统一管理（cc-mcp，仅 Claude Code）

> 依据 `design.md`。纯新增，不改地基任务的 tool_takeover / snapshot / common config。Rust 改动集中在新文件 + 1 处迁移 + 2 处接线（router / mod）。

## 验证命令

| 范围 | 命令 |
|---|---|
| Rust 编译 | `cargo build --manifest-path src-tauri/Cargo.toml` |
| Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml --lib` |
| 格式+lint | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` |
| 前端 | `npm run build && npm test` |

## 有序实现清单（已完成 2026-07-07）

### 阶段 0：Windows 包装 + WSL 检测 + 校验 ✅
- [x] `services/mcp/mod.rs` + `services/mcp/validation.rs`：移植 `validate_server_spec`。
- [x] `services/mcp/claude.rs`：移植 `wrap_command_for_windows`（`#[cfg(windows)]` + `WINDOWS_WRAP_COMMANDS`）+ `is_wsl_path`。
- [x] 单测覆盖全分支（npx/已 cmd/http/python/.cmd 后缀/大小写/wsl$/wsl.localhost/非 WSL）。

### 阶段 1：DB 迁移 v9 + DAO ✅
- [x] `db/migrations.rs` version=9 `create_mcp_servers`（DDL only，`IF NOT EXISTS`）。
- [x] `db/dao/mcp_servers.rs`：`McpServerRow`/`NewMcpServer`/`McpServerUpdate` + `list/get/create/update/delete/list_enabled_claude/upsert`。
- [x] `db/dao/mod.rs` 声明模块。
- [x] 单测：CRUD 往返、enabled 过滤、upsert 存在/不存在两路径。

### 阶段 2：全量投影 + 反向导入 service ✅
- [x] `sync_enabled_to_claude(db)` / `sync_enabled_to_path`（读 live → enabled → 包装 → 替换 mcpServers → atomic_write，保留其它顶层键）。
- [x] `import_from_claude(db)` / `import_from_path`（逐项校验 → upsert → ImportReport）+ `get_status()`。
- [x] 复用 `tool_takeover::atomic_write`（已 pub）。
- [x] **hermetic 修正**：`should_sync_for_path` 由 path 自身 parent 推导兄弟 `.claude` 目录（生产语义正确 + 测试隔离，不看真实 home）。
- [x] 单测：全量投影（启用投影/禁用不投影/未入库项被抹/其它顶层键保留/根非对象报错不覆盖/未安装跳过）；导入（手加入库/已存在只启用不覆盖富信息/非法跳过/缺文件 no-op）。

### 阶段 3：HTTP API ✅
- [x] `http/api/mcp.rs`：list/get/create/update/delete/sync/import/status。写操作成功后调 `sync_enabled_to_claude`。
- [x] `http/api/mod.rs` `pub mod mcp;`；`http/router.rs` `.nest("/api/mcp", ...)`。
- [x] 校验失败 400、根非对象 400（`map_sync_error`）、IO 500。

### 阶段 4：前端 ✅
- [x] `src/lib/api.ts`：`mcpApi` + `McpServer`/`CreateMcpBody`/`UpdateMcpBody`/`McpImportReport`/`McpStatus`。
- [x] `src/pages/McpPage.tsx`：列表 + CRUD 弹窗（裸 JSON 规范编辑 + name/description）+ 启用开关 + 从 live 导入 + status。
- [x] `App.tsx` 路由 `/mcp`；`AppShell.tsx` 导航项「MCP」🧩。

## 风险文件 / 回滚点

- `db/migrations.rs` v9：纯建表，回滚 = 删表（无数据丢失）。
- `~/.claude.json` 写入：根非对象时 400 不覆盖；未安装时 no-op 不建文件；atomic_write 防半写。
- 全量投影抹手加 server：靠反向导入 + 文档提示缓解。
- 回归：`cargo test` 全量（确认 tool_takeover/切换 0 影响）+ `npm test`。

## 完成前检查

- [ ] 五条验证命令全绿。
- [ ] AC1-AC9 单测/UI 逐项覆盖（投影/校验/包装/WSL/导入/未安装跳过/边界 grep）。
- [ ] grep 确认未新增对 `settings.json`/`CLAUDE.md`/`skills/` 的写入。
- [ ] `~/.claude.json` 根非对象/未安装两路径有专项测试。
