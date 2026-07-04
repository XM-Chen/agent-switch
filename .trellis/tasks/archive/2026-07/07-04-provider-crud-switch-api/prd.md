# Provider CRUD 与切换 HTTP API

## Goal

新建 `http/api/providers.rs` 挂 `/api/providers`，把子任务 1/2 的能力（providers DAO + 双模式接管）通过 HTTP API 暴露成可用功能。核心是 `POST /api/providers/{id}/switch`：设 is_current + 按 provider.mode 调 `tool_takeover::enable`（proxy）或 `enable_direct`（direct）。

## Requirements

### R1 新模块 http/api/providers.rs
- `routes()` 返回 `Router<Arc<AppState>>`，遵循 accounts.rs 风格（请求/响应 derive、`Result<_, (StatusCode, String)>` 错误映射）。
- 路由：
  - `GET /api/providers?app_type=<claude-code|codex>` — 列出该 app_type 的 provider，按 sort_index 升序（app_type 缺省报 400）。
  - `POST /api/providers` — 创建 provider（不激活；is_current 恒 0）。
  - `GET /api/providers/{id}` — 单个。
  - `PUT /api/providers/{id}` — 更新部分字段（不含 is_current）。
  - `DELETE /api/providers/{id}` — 删除；若删除的是 current，先 clear_current + 清 tool_takeover.active_provider_id。
  - `POST /api/providers/{id}/switch` — 切换：设 is_current + 按 mode 接管。
  - `POST /api/providers/reorder` — 批量更新 sort_index。

### R2 响应/请求类型
- `ProviderResponse`（含 id/app_type/name/mode/settings_config(JSON Value)/is_current/category/sort_index/notes/created_at/updated_at；meta 解析为 JSON Value）。
- `CreateProviderRequest`（app_type/name/mode/settings_config/category/notes；mode 缺省 "proxy"；sort_index 自动 next）。
- `UpdateProviderRequest`（name/mode/settings_config/category/notes，嵌套 Option 语义）。
- `SwitchResponse { warnings: Vec<String> }`。
- `ReorderRequest { items: Vec<{id, sort_index}> }`。

### R3 切换正确性（核心）
- `switch` 流程：
  1. 查目标 provider（不存在 → 404）。
  2. 解析 app_type + mode。
  3. **先设 is_current**（`providers::set_current`，DB partial unique index 保证互斥）。
  4. 按 mode 接管：proxy → `tool_takeover::enable`；direct → `tool_takeover::enable_direct(provider, crypto)`。
  5. **接管失败必须回滚 is_current**（恢复到切换前的 current，或 clear）——不能出现"DB 说 current 是 A 但工具配置没接管"的不一致。
  6. 返回 warnings（如 crypto 不可用、备份跳过等非致命信息）。
- app_type 与 tool 的映射：claude-code→Tool::ClaudeCode，codex→Tool::Codex。
- 不支持 takeover 的 app_type（未来 opencode 等）→ 400。

### R4 router 注册
- `http/api/mod.rs` 加 `pub mod providers;`。
- `router.rs` 加 `.nest("/api/providers", api::providers::routes())`，且必须在 `/api/{*path}` 兜底之前。
- **注意**：`/api/providers/reorder` 是固定段，需与 `/{id}` 区分——reorder 路由先注册或用不同路径（如 `/reorder` 在 `/{id}` 之前，axum 精确段优先）。

### R5 约束
- 不动代理管道（子任务 4）、不动前端（子任务 5）。
- 遵循现有错误风格与 accounts.rs 模式。
- settings_config 存储为 JSON 字符串（DAO 层），API 层收发 JSON Value。

## Acceptance Criteria

- [ ] `GET /api/providers?app_type=claude-code` 返回按 sort_index 排序的列表；缺 app_type 参数报 400。
- [ ] `POST /api/providers` 创建成功，is_current=0，sort_index 自动追加。
- [ ] `PUT/DELETE/GET {id}` 正常；删除 current provider 时同步清 tool_takeover.active_provider_id。
- [ ] `POST /{id}/switch`（proxy provider）：设 is_current + 写代理接管配置 + tool_takeover mode=proxy。
- [ ] `POST /{id}/switch`（direct provider）：设 is_current + 写直连配置 + tool_takeover mode=direct + active_provider_id。
- [ ] 切换时接管失败：is_current 回滚，DB 状态与工具配置一致（无部分提交）。
- [ ] `POST /reorder` 批量更新 sort_index，列表顺序随之变化。
- [ ] reorder 路由不被 `/{id}` 吞掉（路由注册顺序正确）。
- [ ] 全量门禁：fmt / clippy -D warnings / cargo test 全绿；API 层测试覆盖 switch 成功 + 回滚。

## Notes

- switch 的回滚是本任务最高风险点——参考子任务 2 的 direct→proxy 语义，回滚要恢复到切换前状态。
- crypto 从 `state.crypto.as_deref()` 取；direct 切换时 crypto 不可用应报错（不静默降级）。
- API 层测试用内存 DB + 测试 CryptoService，参考 http/proxy/integration_tests.rs 的 helper 模式。
- data_dir 从 state.data_dir 取；测试用临时目录。
