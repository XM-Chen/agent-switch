# spec 更新、全量门禁与归档

## Goal

收尾双模式切换内核父任务：补齐 providers 数据模型 spec，跑全量收敛门禁，归档最后一个子任务与父任务。

## Requirements

- 在 `spec/backend/database-guidelines.md` 新增 providers 表数据模型约定：列语义（id/app_type/name/mode/settings_config/is_current/category/sort_index/notes/meta/timestamps）、`mode='proxy'|'direct'`、`is_current` 由 `idx_providers_current` partial unique index（`ON providers(app_type) WHERE is_current = 1`）保证同 app_type 互斥、`settings_config` 存接管所需的加密/明文配置（direct 模式含 crypto 加密的 API key）。
- 「双模式接管语义」与「is_current 互斥规则」已在 `http-proxy-guidelines.md` 沉淀，本任务不重复，仅在 database spec 里交叉引用。
- 跑全量门禁：`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test --lib`；`npm run build` / `npm run test`。全绿。
- 记录 session journal（收尾批次）。
- 归档 `07-04-spec-gates-archive` 与父任务 `07-04-dual-mode-switching-core`。

## Acceptance Criteria

- [ ] `database-guidelines.md` 新增 providers 数据模型章节，含 partial unique index 互斥约定
- [ ] `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test --lib` 全绿
- [ ] `npm run build` / `npm run test` 全绿
- [ ] journal 记录本次收尾
- [ ] `07-04-spec-gates-archive` 与 `07-04-dual-mode-switching-core` 均归档至 `archive/2026-07/`

## Notes

- 这是父任务的最后一个子任务，前 5 个子任务（schema-migration / dual-mode-takeover / provider-crud-switch-api / proxy-providers-bridge / frontend-switcher-page）已归档。
- 轻量任务，PRD-only，无需 design.md / implement.md。
