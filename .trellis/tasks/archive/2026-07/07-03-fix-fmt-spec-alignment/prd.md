# cargo fmt 收敛与 spec 错位修正

## Goal

收敛 `cargo fmt` 格式漂移、重构 `.trellis/spec/` 树以修正"backend/frontend index 描述 Trellis 运行时/平台适配层而非 agent-switch 应用本体"的错位,并处理剩余散落 P2/P3(P2-9 retry backoff 已在 proxy 子任务、P2-20 paths 回退、P3 跨子任务死代码归集)。

## Background

- 审计报告 §7 已知限制:`cargo fmt --check` 当前失败(约 10 处格式漂移);spec 层错位:`.trellis/spec/backend|frontend` 的 index 描述 Trellis 工具层而非 agent-switch 应用本体,仅 `.trellis/spec/guides/app-stack-conventions.md` 是应用相关内容。
- 本子任务在所有其他子任务的代码修改完成后**最后执行**,统一 `cargo fmt` 收敛,避免与其他子任务的 Rust 改动互相覆盖。

## Requirements

### cargo fmt 收敛

- R1:`cargo fmt --check` 在 `src-tauri/` 全量通过。
- R2:`cargo clippy --all-targets -- -D warnings` 通过。
- R3:`cargo check` 0 warning。
- 执行时机:其他子任务代码合并后统一运行 `cargo fmt`,一次性收敛所有格式漂移。

### spec 树重构

- R4:`.trellis/spec/backend/index.md` 描述 agent-switch Rust 后端(HTTP 服务、proxy、translator、db、services),而非 Trellis Python 运行时。
- R5:`.trellis/spec/frontend/index.md` 描述 agent-switch React 前端(页面、组件、TanStack Query、API 客户端),而非 Trellis 平台适配层。
- R6:Trellis 运行时/平台适配层规范(Python 脚本、agent 定义、hooks、platform adapters)迁移到独立位置(如 `.trellis/spec/trellis-runtime/` 或 `platform/`),或明确分区保留但 index 重命名为"trellis runtime"而非"backend"。
- R7:`guides/app-stack-conventions.md` 作为应用本体核心规范保留,并在 backend/frontend index 中被引用。
- R8:其他子任务在本任务执行前学到的约定已写入 `guides/app-stack-conventions.md`;本任务将其归位到 backend/frontend 专题文件。

### 散落 P2/P3 归集

- **P2-20** `config/paths.rs:17` app_data_dir 回退 '.' 在 HOME/USERPROFILE 缺失时用 CWD:改为明确的 error 或用户级 fallback(如 dirs::data_dir() None 时返回 error 而非 '.')。归入本任务因 paths.rs 跨子系统且属配置层。
- P2-9 retry backoff 由 proxy 子任务处理。
- 跨子任务死代码归集:其他子任务删除各自范围内的死代码;本任务只做 fmt + spec + paths.rs。

## Design

### spec 重构方案

当前:
```
.trellis/spec/
├── backend/          # 错位:描述 Trellis Python 运行时
├── frontend/         # 错位:描述 Trellis 平台适配层
└── guides/
    ├── app-stack-conventions.md  # 应用本体(正确但孤立)
    ├── code-reuse-thinking-guide.md
    ├── cross-layer-thinking-guide.md
    └── project-conventions.md
```

目标:
```
.trellis/spec/
├── backend/          # 重写:agent-switch Rust 后端
│   ├── index.md      # 重写为应用后端
│   └── *.md          # 应用后端专题(http/db/proxy/translator/services)
├── frontend/         # 重写:agent-switch React 前端
│   ├── index.md      # 重写为应用前端
│   └── *.md          # 应用前端专题(pages/components/api/state)
├── guides/           # 保留:思考指南 + app-stack-conventions
└── trellis-runtime/  # 迁移:原 backend/frontend 中的 Trellis 运行时/平台内容
    └── index.md
```

注意:原 backend/frontend 下 `database-guidelines.md` 等内容若描述的是 Rust 应用 DB 则保留并归入新 backend;若描述 Trellis Python 状态文件则迁入 trellis-runtime。需逐文件判断。

### paths.rs 回退策略

```rust
fn dirs_or_fallback() -> Result<PathBuf, String> {
    if let Some(d) = dirs::data_dir() {
        return Ok(d.join("agent-switch"));
    }
    // 不再用 CWD '.',返回明确 error 让上层提示用户
    Err("无法确定应用数据目录:系统 data_dir 不可用且 HOME/USERPROFILE 缺失".to_string())
}
```
调用方需处理 Result,启动失败时在 UI/log 提示。

## Acceptance Criteria

- [ ] AC1:`cargo fmt --check` 通过
- [ ] AC2:`cargo clippy --all-targets -- -D warnings` 通过
- [ ] AC3:`cargo check` 0 warning
- [ ] AC4:`.trellis/spec/backend/index.md` 描述 agent-switch Rust 后端
- [ ] AC5:`.trellis/spec/frontend/index.md` 描述 agent-switch React 前端
- [ ] AC6:Trellis 运行时/平台适配层规范迁移到独立位置
- [ ] AC7(P2-20):paths.rs 不再用 CWD '.',缺目录时返回明确 error
- [ ] AC8:其他子任务学到的约定已归位到 backend/frontend 专题文件

## Out of Scope

- 跨协议翻译全接线
- 其他子任务范围内的代码修复(各自由其子任务处理)
