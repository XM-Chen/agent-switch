# providers 数据模型与迁移 v7

## Goal

迁移 v7 新建 `providers` 表，作为 ccs 式「可切换单元」的持久化载体，与现有 `accounts`+`endpoints` 并存。同批新建 `db/dao/providers.rs`（CRUD + is_current 互斥 + sort_index）与 `services/provider/mod.rs`（`Provider` 领域类型 + `AppType` 枚举 + `ProviderMeta`）。本子任务是 P1 双模式切换内核的地基，只建数据模型层，不接 HTTP、不改接管、不动代理管道。

## Requirements

### R1 迁移 v7：providers 表
- 在 `src-tauri/src/db/migrations.rs` 的 `MIGRATIONS` 数组**末尾追加** v7（不得改动 v1–v6）。
- 表结构：
  ```sql
  CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    app_type TEXT NOT NULL,              -- 'claude-code' | 'codex'
    name TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'proxy',  -- 'proxy' | 'direct'
    settings_config TEXT NOT NULL,       -- JSON 字符串
    is_current INTEGER NOT NULL DEFAULT 0,
    category TEXT,                       -- official/third_party/aggregator/custom
    sort_index INTEGER,
    notes TEXT,
    meta TEXT NOT NULL DEFAULT '{}',     -- JSON 字符串
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
  );
  CREATE UNIQUE INDEX IF NOT EXISTS idx_providers_current
    ON providers(app_type) WHERE is_current = 1;
  CREATE INDEX IF NOT EXISTS idx_providers_app_sort
    ON providers(app_type, sort_index);
  ```
- partial unique index 是 is_current 互斥的 DB 级兜底：每个 app_type 至多一条 `is_current = 1`。

### R2 DAO 层 `db/dao/providers.rs`
- `ProviderRow`（读）、`NewProvider`（插入）、`ProviderUpdate`（部分更新，用嵌套 `Option` 区分「不更新」与「更新为 NULL」，对齐现有 `accounts.rs`/`endpoints.rs` 风格）。
- 方法：
  - `insert(db, NewProvider) -> Result<(), String>`
  - `get(db, id) -> Result<Option<ProviderRow>, String>`
  - `list_by_app(db, app_type) -> Result<Vec<ProviderRow>, String>`（按 `sort_index ASC, created_at ASC, id ASC` 排序，对齐 endpoints 排序习惯）
  - `get_current(db, app_type) -> Result<Option<ProviderRow>, String>`
  - `set_current(db, app_type, id) -> Result<(), String>`：**事务内**先把该 app_type 全部 `is_current=0`，再把目标 `is_current=1`；目标不存在或 app_type 不匹配则报错回滚。
  - `update(db, id, ProviderUpdate) -> Result<(), String>`
  - `delete(db, id) -> Result<(), String>`
  - `next_sort_index(db, app_type) -> Result<i64, String>`：返回 `MAX(sort_index)+1`，空表返回 0。
- 时间戳用 `time::OffsetDateTime` ISO8601（对齐现有 DAO）。

### R3 领域层 `services/provider/mod.rs`
- `AppType` 枚举：成员 `ClaudeCode`、`Codex`。`as_str()`（`"claude-code"`/`"codex"`）、`from_str()`、`all()`。为未来扩展预留（新增成员即可），但 P1 仅两种。
- `ProviderMode` 枚举：`Proxy`、`Direct`，`as_str`/`from_str`，默认 `Proxy`。
- `Provider` 领域结构：`id/app_type/name/mode/settings_config(serde_json::Value)/is_current/category/sort_index/notes/meta(ProviderMeta)/created_at/updated_at`。提供 `from_row`（ProviderRow → Provider，解析 JSON 字段）。
- `ProviderMeta`：JSON 承载的元数据结构，字段全 `Option` + `#[serde(skip_serializing_if)]`，`camelCase`。P1 先放最小集（如 `endpoint_group: Option<String>` 预留 proxy 模式圈定端点子集），可为空 `{}`。
- 模块挂到 `services/mod.rs`。

### R4 约束
- 不改动 v1–v6 迁移。
- 不新增 HTTP 路由、不改 `tool_takeover`、不动代理管道（那些是子任务 2/3/4）。
- 遵循现有错误风格：`Result<T, String>`，中文错误消息。
- 遵循 `.trellis/spec/backend/` 数据库与目录规范。

## Acceptance Criteria

- [ ] 迁移 v7 追加在数组末尾，`cargo test` 中现有 `fresh_db_runs_all_migrations_in_order` 与 `migration_versions_are_ascending` 仍通过，且新增断言验证 providers 表与两个索引存在。
- [ ] `set_current` 互斥：给同一 app_type 连续 set 两个不同 provider，只有最后一个 `is_current=1`，其余为 0（单元测试覆盖）。
- [ ] partial unique index 生效：手动向同 app_type 插入两条 `is_current=1` 应被 DB 拒绝（单元测试覆盖）。
- [ ] `next_sort_index` 在空表返回 0，插入后追加递增（单元测试覆盖）。
- [ ] `mode` 缺省为 `'proxy'`（单元测试：NewProvider 不显式给 mode 时落库为 proxy）。
- [ ] `ProviderUpdate` 部分更新语义正确：只更新提供的字段，未提供的保持不变（单元测试覆盖）。
- [ ] `from_row` 正确解析 settings_config 与 meta 的 JSON。
- [ ] 全量门禁通过：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

## Notes

- 本子任务是纯数据模型层，交付后子任务 2（接管改造）、3（HTTP API）才能接线。
- providers 与 endpoints 的职责边界见父任务计划：providers 是切换面，endpoints 是代理路由底座。
- 参考现有 `db/dao/accounts.rs`、`db/dao/endpoints.rs` 的 Row/New/Update 三件套与嵌套 Option 语义。
