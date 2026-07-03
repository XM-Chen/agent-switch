# 模型管理刷新与别名实现计划

## 前置条件

- 父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`
- 已归档子任务：app-shell-local-service, accounts-endpoints-credential-security
- 不开始实现，直到用户 review 并明确同意。

## 实现顺序

### 1. 迁移 v3：endpoint_models + model_aliases 表

在 `migrations.rs` 的 MIGRATIONS 数组追加 v3。

### 2. DAO 层

新增：
- `dao/endpoint_models.rs`：CRUD + 按 endpoint/capability/source 查询 + sync 覆盖 upsert
- `dao/model_aliases.rs`：CRUD + 按 scope 查询 + 按 alias_name+scope 查询

### 3. 刷新服务

新增：
- `services/model_sync.rs`：
  - `SyncService`：startup 刷新、手动刷新、定时调度
  - `fetch_models_from_endpoint(endpoint)`：HTTP GET {base_url}/v1/models
  - `sync_one_endpoint(endpoint, db)`：获取模型 → upsert → 标记下线
  - `RefreshThrottle`：host-level + credential-level 并发控制

依赖：`reqwest`（已有）、`tokio::time::interval`（定时）、`rand`（jitter）

### 4. 别名引擎

新增：
- `services/model_alias.rs`：
  - `AliasEngine`：读取数据库 → 按优先级解析
  - `ResolutionContext` + `ResolvedAlias`

### 5. 管理 API

新增：
- `http/api/models.rs`：`/api/models/*`
- `http/api/aliases.rs`：`/api/models/aliases/*`

更新 `router.rs` 挂载。

### 6. 刷新设置

使用 `app_metadata` 表或 settings JSON 保存 `auto_model_refresh_enabled`。
不需要新增迁移，已有 `endpoints` 表可复用。

### 7. 前端模型页面

更新 `ModelsPage.tsx`：模型表格、刷新按钮、自定义模型表单、别名面板。

### 8. 验证

```bash
npm run build
cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings
./src-tauri/target/debug/agent-switch.exe &
curl -s http://127.0.0.1:42567/api/models
curl -s -X POST http://127.0.0.1:42567/api/models/sync
curl -s -X POST http://127.0.0.1:42567/api/models/aliases -H "Content-Type: application/json" -d '{"scope_type":"global","scope_id":"","alias_name":"test-alias","target_endpoint_id":"...","target_model_name":"gpt-4","priority":0}'
curl -s http://127.0.0.1:42567/api/models/resolve/test-alias
```

## 风险与回滚点

- 刷新访问第三方 `/v1/models` 可能失败（网络/限流）→ 失败不影响已有模型，只记录 error。
- endpoint_models 在刷新中被覆盖可能导致 UI 闪烁 → 前端使用 TanStack Query 缓存。
- 别名解析在大批量 alias 时性能 → 第一版不设硬限制；如需要可加 DB index。
