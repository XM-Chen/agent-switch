# 实现计划：Prompts 管理（cc-prompts，仅 Claude Code）

> 依据 `design.md`。锚定 cc-mcp 已落地范式：独立表 + DAO + service + REST + 前端 page。纯新增，不改地基 tool_takeover / snapshot / common config。

## 验证命令

| 范围 | 命令 |
|---|---|
| Rust 编译 | `cargo build --manifest-path src-tauri/Cargo.toml` |
| Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml --lib` |
| 格式+lint | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` |
| 前端 | `npm run build && npm test` |

## 有序实现清单

### 阶段 0：路径 + should_sync + 单激活原子性 ✅（实现时勾）
- [ ] `services/prompts/mod.rs` + `services/prompts/claude.rs`：`claude_md_path()` + `should_sync_for_path`（hermetic，由 path parent 推导，不看真实 home）。
- [ ] `db/dao/prompts.rs`：`set_enabled_exclusive`（同事务：目标置 1、其余置 0）。
- [ ] 单测：单激活不变量（启用 B → 仅 B enabled；启用 A→B→A → 仅 A）。

### 阶段 1：DB 迁移 v10 + DAO ✅
- [ ] `db/migrations.rs` version=10 `create_prompts`（DDL only，`IF NOT EXISTS`）。
- [ ] `db/dao/prompts.rs`：`PromptRow`/`NewPrompt`/`PromptUpdate` + `list/get/create/update/delete/get_enabled_claude/set_enabled_exclusive`。
- [ ] `db/dao/mod.rs` 声明模块。
- [ ] 单测：CRUD 往返、enabled 查询、set_enabled_exclusive 事务原子性。

### 阶段 2：单激活 + 回填 + 投影 service ✅
- [ ] `enable_prompt(db, id)`：回填保护两分支（有启用项回填进该项 / 无启用项建 backup，重复内容跳过）→ set_enabled_exclusive → should_sync 通过则原子写 live。
- [ ] `disable_prompt(db, id)`：置 0；无启用项则清空 live（写空串）。
- [ ] `delete_prompt(db, id)`：激活项拒删。
- [ ] `import_from_claude(db)` + `import_on_first_launch(db)`（幂等：DB 非空跳过）+ `get_status(db)`。
- [ ] 复用 `tool_takeover::atomic_write`（已 pub）。
- [ ] 单测：单激活投影、回填往返、回填两分支、重复备份跳过、禁用清空、删除保护、反向导入、首次自动导入幂等、未安装跳过。

### 阶段 3：HTTP API ✅
- [ ] `http/api/prompts.rs`：list/get/create/update/delete/enable/disable/import/status。
- [ ] `http/api/mod.rs` `pub mod prompts;`；`http/router.rs` `.nest("/api/prompts", ...)`。
- [ ] 错误映射：删除激活项 400、校验失败 400、IO 500（对齐 cc-mcp `map_sync_error`）。

### 阶段 4：前端 ✅
- [ ] `src/lib/api.ts`：`promptsApi` + `Prompt`/`CreatePromptBody`/`UpdatePromptBody`/`PromptImportReport`/`PromptStatus`。
- [ ] `src/pages/PromptsPage.tsx`：列表 + CRUD 弹窗（content 文本域 + name/description）+ 激活开关（单选语义）+ 从 live 导入 + status。
- [ ] `App.tsx` 路由 `/prompts`；`AppShell.tsx` 导航项「Prompts」。

### 阶段 5：首次自动导入挂载 ✅
- [ ] 在应用启动初始化（DB 迁移完成后）调用 `prompts::import_on_first_launch(db)`（幂等兜底，失败仅告警不阻断）。
- [ ] 确认挂载函数位置（design 标注，实现时锁定到既有 setup 序列）。

## 风险文件 / 回滚点

- `db/migrations.rs` v10：纯建表，回滚 = 删表（无数据丢失）。
- `~/.claude/CLAUDE.md` 写入：未安装时 no-op 不建文件；atomic_write 防半写。
- 单激活事务性：`set_enabled_exclusive` 同事务，单测验证至多一份激活。
- 首次导入幂等：DB 非空跳过，单测验证启动不重复导入。
- 回归：`cargo test` 全量（确认 tool_takeover/切换/cc-mcp 0 影响）+ `npm test`。

## 完成前检查

- [ ] 四条验证命令全绿。
- [ ] AC1-AC10 逐项覆盖（单激活/回填两分支/禁用清空/删除保护/导入/首次自动导入幂等/未安装跳过/解耦 grep/回归）。
- [ ] grep 确认未新增对 `settings.json`/`~/.claude.json`/`skills/`/`projects/` 的写入。
- [ ] `~/.claude` 不存在时 enable/disable 两路径有专项测试。
