# 实现计划：切换语义增强（A1-hybrid + B1）

> 依据 `design.md`。有序清单，每步可独立验证。Rust 侧改动集中在 `src-tauri/src/`，迁移走 `db/migrations.rs`（当前 schema 版本 v8，本任务加 v9）。

## 验证命令

| 范围 | 命令 |
|---|---|
| Rust 编译 | `cargo build --manifest-path src-tauri/Cargo.toml` |
| Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml` |
| 前端类型+构建 | `npm run build`（`tsc --noEmit && vite build`） |
| 前端测试 | `npm test`（`vitest run`） |

每个阶段结束后至少跑对应侧的编译/测试；全部完成后四条全绿。

## 设计修正（实现期，见 design.md「关键设计修正」）

原清单基于「重定义 `settings_config` 为全文 + DB 迁移 v9 + `${ENDPOINT}` 占位符 + resolve/redact」方案。
实现期重读调用图后**改用 `meta.snapshot` 承载全文快照**，blast radius 更小：
- `settings_config` **不变**（仍是连接规格）→ 无 DB schema 迁移、Codex/存量 provider/既有测试零风险。
- token 天然不落库：快照存前删连接层 env（`strip_connection_env`），无需 `${ENDPOINT}` 占位符 / resolve / redact。
- 连接层复用既有 `apply`/`apply_direct` 注入 live。

下方清单已按实际实现重写并勾选。

## 有序实现清单

### 阶段 0：deep-merge/strip 工具（无依赖，先落地 + 单测）✅
- [x] 新增 `src-tauri/src/services/tool_takeover/json_merge.rs`：`deep_merge` / `deep_remove` / `is_subset`。移植 ccs `live.rs:51-143`。
- [x] 单测覆盖：嵌套对象合并、数组整体替换、subset 检测、remove 后空对象清理。
- [x] 验证：`cargo test json_merge`（全绿）

### 阶段 1：快照层数据模型（无 DB 迁移，复用 meta JSON + app_metadata kv）✅
- [x] `settings_config` **不改**；快照存 `meta.snapshot`、三态存 `meta.common_config_enabled`（`claude_snapshot.rs` 读写辅助）。
- [x] common config 存 `app_metadata` 键 `common_config_claude-code`（复用现有 kv，无新表/新迁移）。
- [x] 存量 provider 无 `meta.snapshot` → 视为空快照，首切回填自身保护既有 live 配置。
- [x] 验证：`cargo test`（snapshot/common_enabled meta 往返用例）

### 阶段 2：连接层剥离（替代原 resolve/redact 占位符方案）✅
- [x] `strip_connection_env`：backfill 存快照前删 `env.ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`，明文 token 永不落库。
- [x] 无需 `${ENDPOINT}` 占位符——连接层每次切换由 mode+endpoint 经既有 `apply`/`apply_direct` 重新注入。
- [x] 单测：`strip_connection_env` 只删连接键、env 空则删除、无 env 为 no-op；backfill 后 meta 无明文 token 断言。
- [x] 验证：`cargo test`

### 阶段 3：write_live_snapshot + backfill + common config 三层（切换链路重写）✅
- [x] `claude_snapshot.rs`：`write_live_snapshot`（整文件覆盖 + 排序键 + 复用 `atomic_write`）、`read_live`、`build_effective`（deep_merge common）。
- [x] `tool_takeover/mod.rs`：`switch_claude` / `switch_claude_at` 编排——
  1. **切走前 backfill**：read_live → `strip_connection_env` → `strip_common`（`deep_remove`）→ 存回上一个 current provider 的 `meta.snapshot`。
  2. **写目标 provider**：`snapshot_from_meta` → `build_effective`（叠加 common，按三态）→ `write_live_snapshot` 整文件覆盖 → `apply`/`apply_direct` 注入连接层。
- [x] proxy/direct 两条链统一走 snapshot 路径；direct 的 base_url/token/model 仍来自 `settings_config` 端点引用（`resolve_direct_config` 不变）。
- [x] `disable`/`reapply` **不变**（Codex 与既有 Claude 路径逐字节保留，只在 switch 路径接入快照）。
- [x] **保留** `backup_before_write` 文件级 `.bak`，与 backfill 并存。
- [x] `providers.rs`：`perform_switch` 闭包扩为 3 参（provider, prev, tool），Claude 走 `switch_claude`、Codex 走旧 `enable`/`enable_direct`。
- [x] 验证：`cargo test tool_takeover`（切走-切回还原 AC4、common config 三态 AC5、strip AC6、token 不落盘 AC7、proxy/direct 往返、备份 AC3）全绿。

### 阶段 4：common config 后端 API + 三态读写 ✅
- [x] `read_common_config` / `write_common_config`（`app_metadata` kv，须为对象）。
- [x] HTTP API：`GET/PUT /api/common-config/{tool}`（裸 JSON，仅 claude-code；`http/api/common_config.rs` + router 注册）。
- [x] `provider.meta.common_config_enabled` 三态并入 provider update（`common_enabled_into_meta`，以现有 meta 为基底保留 snapshot）。
- [x] 验证：`cargo test`（common config 往返 + 三态 meta 用例）全绿。

### 阶段 5：前端最小接线 ✅
- [x] `src/lib/api.ts`：`commonConfigApi.get/set` + `UpdateProviderBody.common_config_enabled`（嵌套 Option 语义）。
- [x] 验证：`npm run build`（tsc + vite 全绿）、`npm test`（45 全绿）。

### 阶段 6：ccs 导入器适配 —— 本批不需要（设计修正后 `settings_config` 语义未变）
- [x] design.md 已明确：`settings_config` 不变 → ccs 导入器**不改**。ccs settings.json 全文里的非连接键（hooks 等）导入 `meta.snapshot` 作为**后续增强**，非本批交付。

## 风险文件 / 回滚点

- `db/migrations.rs` v9 数据迁移：**最高风险**。存量 provider 的 `settings_config` 语义变更，迁移写错会让用户切换后配置错乱。回滚点：迁移前自动 DB 备份（确认 agent-switch 是否已有；无则本任务加）。迁移必须幂等 + 有存量数据单测。
- `tool_takeover/mod.rs` 切换链路重写：影响所有切换。保留旧 `apply` 路径到新路径测试通过前，灰度切换。
- token 脱敏 `redact_tokens`：若脱敏遗漏，明文 token 会落 `settings_config`（安全回归）。单测必须覆盖"backfill 后 DB 里绝无明文 token"。
- `.bak` 文件级备份保留，作为整体回滚兜底。

## 完成前检查
- [ ] 四条验证命令全绿（cargo build / cargo test / npm run build / npm test）。
- [ ] 存量数据迁移用例通过（旧端点引用 → 全文+endpoint_ref）。
- [ ] 安全断言：切换/backfill 后 `settings_config` 与 `common_config` 中**无明文 token**（专项测试）。
- [ ] direct 与 proxy 模式切换往返均正确，direct 不静默降级 proxy。
- [ ] 被依赖子任务（cc-prompts / cc-common-config-editor）所需的 common config 读写 API 已就绪。
