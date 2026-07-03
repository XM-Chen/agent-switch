# cargo fmt 收敛与 spec 错位修正 - Implement

## 执行前置条件

本任务必须最后执行。启动本任务前确认:

- `07-03-fix-translator-wire-format` 已完成并归档/提交
- `07-03-fix-proxy-oauth-failover` 已完成并归档/提交
- `07-03-fix-db-portability` 已完成并归档/提交
- `07-03-fix-codex-oauth-credentials` 已完成并归档/提交
- `07-03-fix-frontend-deadcode-tests` 已完成并归档/提交

原因:本任务会统一运行 `cargo fmt`,并重构 spec tree;如果提前执行,其他 Rust 子任务会再次引入格式漂移,其他子任务的新增约定也无法归位。

## Phase 2 实施步骤

### Step 1: 启动前检查

```bash
python ./.trellis/scripts/get_context.py
python ./.trellis/scripts/task.py list --mine
```

确认当前活动任务为:

```text
.trellis/tasks/07-03-fix-fmt-spec-alignment
```

确认其他 5 个 sibling 子任务已不在 planning/in_progress 状态。

### Step 2: 修复 paths.rs fallback

目标文件:

- `src-tauri/src/config/paths.rs`
- 可能涉及: `src-tauri/src/lib.rs` 中 app data dir fallback 调用点

实施:

1. 删除 `unwrap_or_else(|_| ".".to_string())` 逻辑。
2. 改为不会污染 CWD 的 fallback 链:
   - `dirs::data_dir()`
   - Windows: `APPDATA` / `LOCALAPPDATA`
   - Unix: `HOME/.local/share`
   - 最后: `std::env::temp_dir().join("agent-switch-data")`
3. 增加单元测试覆盖:
   - data dir 正常时不测试系统环境细节
   - 辅助函数可注入 env resolver 时覆盖 HOME/USERPROFILE 缺失不会返回 `.`
4. 若函数不好测,提取纯函数:

```rust
fn dirs_or_fallback_with_env<F>(data_dir: Option<PathBuf>, get_env: F, temp_dir: PathBuf) -> PathBuf
where
    F: Fn(&str) -> Option<std::ffi::OsString>
```

### Step 3: 迁移 Trellis runtime spec

创建目录:

```text
.trellis/spec/trellis-runtime/
```

迁移/合并原规范:

| 原路径 | 目标路径 |
|--------|----------|
| `.trellis/spec/backend/directory-structure.md` | `.trellis/spec/trellis-runtime/runtime-directory-structure.md` |
| `.trellis/spec/backend/database-guidelines.md` | `.trellis/spec/trellis-runtime/runtime-persistence.md` |
| `.trellis/spec/backend/error-handling.md` | `.trellis/spec/trellis-runtime/runtime-error-handling.md` |
| `.trellis/spec/backend/logging-guidelines.md` | `.trellis/spec/trellis-runtime/runtime-logging.md` |
| `.trellis/spec/backend/quality-guidelines.md` | `.trellis/spec/trellis-runtime/runtime-quality.md` |
| `.trellis/spec/frontend/*` 中平台适配内容 | `.trellis/spec/trellis-runtime/platform-*.md` |

注意:

- 可用 `git mv` 保留历史。
- 如果部分文件内容过细,可先迁移并保留原文,再更新标题/相对链接。
- 不要删除 Trellis runtime 规范。

### Step 4: 重写 backend 应用规范入口

重写/创建:

- `.trellis/spec/backend/index.md`
- `.trellis/spec/backend/directory-structure.md`
- `.trellis/spec/backend/database-guidelines.md`
- `.trellis/spec/backend/http-proxy-guidelines.md`
- `.trellis/spec/backend/translator-guidelines.md`
- `.trellis/spec/backend/portability-guidelines.md`
- `.trellis/spec/backend/quality-guidelines.md`

内容来源:

- `.trellis/spec/guides/app-stack-conventions.md`
- 审计报告 P1/P2 修复经验
- 当前 `src-tauri/src/` 目录结构

必须包括:

- Rust/Tauri/Axum/SQLite 分层边界
- DAO 事务与 `Arc<Mutex<Connection>>` 约定
- proxy/failover/stream guard 不变量
- translator SSE wire-format 不变量
- portability import/export 安全约定
- paths.rs data dir fallback 约定
- 质量门命令

### Step 5: 重写 frontend 应用规范入口

重写/创建:

- `.trellis/spec/frontend/index.md`
- `.trellis/spec/frontend/directory-structure.md`
- `.trellis/spec/frontend/api-client-guidelines.md`
- `.trellis/spec/frontend/state-management.md`
- `.trellis/spec/frontend/component-guidelines.md`
- `.trellis/spec/frontend/quality-guidelines.md`

内容来源:

- `.trellis/spec/guides/app-stack-conventions.md`
- 当前 `src/` 目录结构
- 前端子任务引入的测试框架/共享工具约定

必须包括:

- 8 页面中文 UI 结构
- `src/lib/api.ts` API client 约定
- TanStack Query queryKey 约定
- 错误态不得静默 fallback 为空数据
- 日志测试/生产过滤语义
- Vitest/前端测试命令

### Step 6: 更新 guides index

更新:

- `.trellis/spec/guides/index.md`

要求:

- 指向新的 backend/frontend 应用规范
- 指向 `trellis-runtime/` 规范
- 保留 `app-stack-conventions.md` 作为总览/历史约定
- 删除或修正“backend/frontend 是 Trellis runtime/platform”的旧描述

### Step 7: 统一 cargo fmt

```bash
cd src-tauri
cargo fmt
cargo fmt --check
```

如果 `cargo fmt --check` 仍失败,查看 diff 并重复。

### Step 8: 质量门

Rust:

```bash
cd src-tauri
cargo check
cargo clippy --all-targets -- -D warnings
```

Frontend:

```bash
npm run build
```

Trellis/spec:

```bash
python ./.trellis/scripts/get_context.py --mode packages
python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform claude
python ./.trellis/scripts/task.py validate 07-03-fix-fmt-spec-alignment
```

若 `task.py validate` 对普通任务名不支持或只验证 context,记录跳过原因。

## 风险与回滚

### 风险 1: spec 文件迁移破坏相对链接

缓解:

- 迁移后使用 `grep -R "\.\./" .trellis/spec` 检查明显断链
- 运行 `get_context.py --mode packages` 确认 spec layer 可发现

### 风险 2: paths.rs fallback 改成 Result 造成调用链扩散

缓解:

- 不改公共签名,采用 temp dir fallback
- 单元测试保证不返回 `.`

### 风险 3: cargo fmt 产生大 diff 混杂功能修复

缓解:

- 本任务最后执行
- `cargo fmt` 单独提交,或至少在 commit message 明确格式收敛

## 完成前检查清单

- [ ] `src-tauri/src/config/paths.rs` 不再回退到 `.`
- [ ] `.trellis/spec/backend/index.md` 是 agent-switch Rust 后端规范
- [ ] `.trellis/spec/frontend/index.md` 是 agent-switch React 前端规范
- [ ] `.trellis/spec/trellis-runtime/index.md` 存在并保留 Trellis runtime/platform 规范入口
- [ ] `.trellis/spec/guides/index.md` 链接更新
- [ ] `cargo fmt --check` 通过
- [ ] `cargo check` 通过且 0 warning
- [ ] `cargo clippy --all-targets -- -D warnings` 通过
- [ ] `npm run build` 通过
- [ ] Trellis context/package 命令通过

## 提交建议

建议拆为 2 个提交:

1. `fix(config): avoid cwd fallback for app data dir`
2. `docs(spec): align spec layers with agent-switch app`
3. `style(rust): apply cargo fmt` 若 fmt diff 很大则单独提交

如果实际 diff 较小,可合并为一个收敛提交。
