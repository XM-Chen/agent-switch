# cargo fmt 收敛与 spec 错位修正 - Design

## 1. 范围与边界

本任务是 `07-03-fix-remaining-defects` 父任务下的最后收敛子任务。它不负责修复 translator / proxy / db / codex-oauth / frontend 子系统自身缺陷,只负责在其他子任务完成后统一收敛以下内容:

1. Rust 格式与质量门:
   - `cargo fmt --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo check` 0 warning
2. `.trellis/spec/` 结构错位修正:
   - `backend/` 应描述 agent-switch Rust 后端,不是 Trellis Python runtime
   - `frontend/` 应描述 agent-switch React 前端,不是平台适配层
   - Trellis runtime / platform adapter 规范迁移到独立 spec 区域
3. P2-20 `config/paths.rs` 数据目录 fallback:
   - 不得在 HOME / USERPROFILE 缺失时回退到当前目录 `.`
   - 必须使用可解释、稳定、不会污染 CWD 的错误或 fallback 策略

## 2. 当前证据

### 2.1 cargo fmt

当前 `cargo fmt --check` 输出约 250 行 diff,集中在近期 Rust 修复文件中,包括但不限于:

- `src-tauri/src/db/dao/endpoint_models.rs`
- `src-tauri/src/http/proxy/mod.rs`
- `src-tauri/src/http/proxy/oauth_refresh.rs`

本任务最后统一运行 `cargo fmt`,避免其他子任务继续修改 Rust 文件后再次产生格式漂移。

### 2.2 spec 错位

当前 `.trellis/spec/backend/index.md` 标题为 `Trellis Runtime Guidelines`,明确写着:

> In this spec layer, "backend" means the local Python runtime under `.trellis/scripts/`...

当前 `.trellis/spec/frontend/index.md` 标题为 `Agent Platform Guidelines`,明确写着:

> This repository does not contain a browser frontend, React components, or CSS assets...

这与当前代码库事实矛盾:项目已经是 Tauri + Rust + React 应用,存在 `src-tauri/` 和 `src/` 前后端实现。

应用本体规范实际集中在 `.trellis/spec/guides/app-stack-conventions.md`,该文件开头已声明:

> 适用范围:agent-switch 桌面应用本体(Tauri + Rust + Web 前端),不是 Trellis 工具层。

因此本任务不是新增全新规范体系,而是把已经存在的应用本体约定归位到正确 spec layer。

## 3. 目标 spec 结构

```text
.trellis/spec/
├── backend/                 # agent-switch Rust 后端规范
│   ├── index.md
│   ├── directory-structure.md
│   ├── database-guidelines.md
│   ├── http-proxy-guidelines.md
│   ├── translator-guidelines.md
│   ├── portability-guidelines.md
│   └── quality-guidelines.md
├── frontend/                # agent-switch React 前端规范
│   ├── index.md
│   ├── directory-structure.md
│   ├── api-client-guidelines.md
│   ├── state-management.md
│   ├── component-guidelines.md
│   └── quality-guidelines.md
├── trellis-runtime/          # Trellis Python runtime + 平台适配规范
│   ├── index.md
│   ├── runtime-directory-structure.md
│   ├── runtime-persistence.md
│   ├── platform-adapters.md
│   ├── hooks-and-context.md
│   └── runtime-quality.md
└── guides/
    ├── app-stack-conventions.md
    ├── code-reuse-thinking-guide.md
    ├── cross-layer-thinking-guide.md
    └── project-conventions.md
```

### 3.1 迁移原则

- 原 `backend/*` 中描述 `.trellis/scripts/`、task.json、workflow hooks 的内容迁移到 `trellis-runtime/`。
- 原 `frontend/*` 中描述 agent definitions、slash commands、platform adapters 的内容迁移到 `trellis-runtime/`。
- 新 `backend/*` 和 `frontend/*` 应从 `guides/app-stack-conventions.md`、实际源码目录、近期修复经验中提炼应用本体规范。
- `guides/app-stack-conventions.md` 可继续保留为总览,但不再是唯一应用规范入口。

### 3.2 兼容性

- 不删除历史规范内容,只迁移/重命名/重写入口,避免丢失 Trellis runtime 规范。
- `.trellis/spec/guides/index.md` 需更新链接,避免指向过期语义。
- 若其他工具通过 `backend` / `frontend` layer 名称加载 spec,迁移后名称更符合当前项目主体,不会破坏应用开发上下文。

## 4. paths.rs 数据目录策略

### 4.1 当前问题

当前实现:

```rust
fn dirs_or_fallback() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
    })
}
```

如果系统 data dir、HOME、USERPROFILE 都不可用,最终使用当前目录 `.`。这会导致数据库位置随启动 CWD 变化,并可能污染用户工作目录。

### 4.2 推荐策略

保持 `app_data_dir()` 对外签名不变的前提下,避免大范围调用方变更:

```rust
fn dirs_or_fallback() -> PathBuf {
    dirs::data_dir()
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| std::env::temp_dir().join("agent-switch-data"))
}
```

并增加 `tracing::warn!` 或调用方日志,说明使用临时目录 fallback。这样不污染 CWD,且不需要把 `app_data_dir()` 改成 `Result<PathBuf, String>` 引发启动链路大改。

如果实现期发现 `tracing` 在 `config/paths.rs` 中不适合使用,则使用纯 fallback + 单元测试覆盖即可。

### 4.3 取舍

- 改成 `Result` 最严格,但会影响 `lib.rs` 早期初始化和测试调用点,变更面大。
- 回退到 temp dir 不如明确失败严格,但满足“不使用 CWD”核心要求,且第一版桌面应用更偏向可启动。
- 本任务采用 temp dir fallback,并在 spec 中记录该降级语义。

## 5. 质量门设计

按顺序执行:

1. 其他 5 个子任务全部完成并提交/合并后,启动本任务。
2. 运行 `cargo fmt`。
3. 运行 `cargo fmt --check`。
4. 运行 `cargo check`。
5. 运行 `cargo clippy --all-targets -- -D warnings`。
6. 运行 `npm run build` 确认 spec/front-end 文件调整未影响前端构建。
7. 运行 Trellis/context 基础检查:
   - `python ./.trellis/scripts/get_context.py --mode packages`
   - `python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform claude`

## 6. 回滚点

- `src-tauri/src/config/paths.rs`:单文件小改,可单独 revert。
- `.trellis/spec/backend/` / `.trellis/spec/frontend/` / `.trellis/spec/trellis-runtime/`:目录级迁移,需在 commit 前检查所有链接。
- `cargo fmt`:可能触及很多 Rust 文件,应在其他代码修复完成后单独提交,便于回滚格式-only diff。

## 7. 验收映射

| PRD AC | Design 对应 |
|--------|-------------|
| AC1 cargo fmt | §5 |
| AC2 clippy | §5 |
| AC3 cargo check | §5 |
| AC4 backend index | §3 |
| AC5 frontend index | §3 |
| AC6 Trellis runtime 迁移 | §3.1 |
| AC7 paths.rs 不用 CWD | §4 |
| AC8 子任务约定归位 | §3 + §5 |
