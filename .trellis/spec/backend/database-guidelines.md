# 数据库与模型同步规范

## SQLite 连接

- `rusqlite::Connection` 通过 `Arc<Mutex<Connection>>` 共享。
- 不要在持有 DB mutex 时等待外部网络 I/O。
- 多表一致性写入必须使用事务。

## DAO 约定

- Handler/service 调用 DAO，不直接拼散落 SQL。
- DAO 返回 `Result<_, String>` 时错误消息必须可诊断。
- 模型能力字段是 JSON 字符串数组；能力过滤必须做 JSON token 级匹配，禁止 `LIKE '%cap%'` 子串匹配。
- 时间戳（`created_at`/`updated_at` 等）统一用 ISO8601 字符串存储。写入时调用共享 helper `crate::db::dao::now_iso() -> Result<String,String>`，**不要在各 DAO 里重复实现 `now_iso`/`iso_now`**（本次精简一次性合并了 11 处副本）。新增 DAO 文件直接 `use crate::db::dao::now_iso;`。

## request_logs

- 请求日志只保存摘要，不保存 prompt/messages/完整 headers/API key/token。
- prune 应按 overflow 删除最旧行，避免 `NOT IN (SELECT ...)` 反连接造成大表 O(N²) 风险。
- **写入侧不走 DAO**：`db/dao/request_logs.rs` 只保留 `list`/`prune_old`/`RequestLogRow`；运行期日志写入由 `http/proxy/logger.rs::write_log` 直接执行，DAO 不再提供 `insert`/`new_log`（曾预留但无生产调用，已删）。如需新增写入路径，复用 `write_log` 而非在 DAO 重建。

## 模型同步

- `/v1/models` 返回空 `data: []` 视为异常/可疑响应，不得批量禁用既有 synced 模型。
- synced upsert 与同端点同名模型冲突时，不得破坏 custom 模型；若当前 schema 只能有一行，则必须明确 source 语义并配测试。
- capabilities 优先使用上游字段；缺失时保守推断，不能把 embeddings/images/audio-only 模型误标为 chat/tool_calling。

## providers 数据模型（ccs 式切换单元）

`providers` 表（迁移 v7 `create_providers`）是 ccs 式「可切换单元」，承载切换面：UI 列出哪些、当前激活哪个。它与既有 `endpoints`+`accounts` 管道**并存**而非替代。

- **列语义**：`app_type`（claude-code/codex/…）、`name`、`mode`（`'proxy'` | `'direct'`）、`settings_config`（JSON 字符串）、`is_current`、`category`（official/third_party/aggregator/custom，对齐 ccs）、`sort_index`、`notes`、`meta`。
- **双模式的 `settings_config` 语义**：`proxy` 模式存「代理指向配置」（工具指向本地代理，上游路由仍由 endpoints 管道决定）；`direct` 模式存「工具原生配置」（含真实凭证，绕过代理直接写工具文件）。
- **加密边界**：providers 表按原样存 JSON，**不在 DAO/表层加密**；direct 模式真实凭证的加解密职责在接管服务（`tool_takeover::enable_direct` + crypto）侧，不在 providers 数据层。
- **激活态互斥**：`is_current` 的按 `app_type` 唯一性由 partial unique index `idx_providers_current`（`ON providers(app_type) WHERE is_current = 1`）在 DB 层兜底并发写入。应用层禁止直接 `UPDATE is_current`——激活必须走 `set_current` 事务（先清同 app_type 旧 current 再置新，单事务内完成）。`create`/`update` 恒不改 `is_current`。
- **排序**：`idx_providers_app_sort`（`app_type, sort_index`）支撑列表稳定排序；新建时 `sort_index` 自动追加（`next_sort_index`），批量 reorder 走单独路径。

> 切换（`POST /api/providers/{id}/switch`）如何联动 `is_current` 与 `tool_takeover`、失败回滚与删除 current 复位，见 http-proxy 规范的「Provider 切换的原子性」。

### `meta` 已约定的子键（Claude Code 快照层）

`meta` 是「不写入 live 的元数据」JSON。Claude Code 的 ccs 式回填保护 + Common Config 三层（A1-hybrid）复用它承载 per-provider 状态，**不新增列、不改 `settings_config` 语义**：

- `meta.snapshot`：per-provider 的 `settings.json`「非连接键」全文快照（hooks/permissions/statusLine/env 内非连接键等）。**永不含连接层**（`env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`）——切走前 backfill 用 `strip_connection_env` 剥离后再存回，因此明文 token 天然不落库，无需占位符/脱敏逻辑。连接层每次切换由 mode+endpoint 经 `apply`/`apply_direct` 重新注入 live。缺省无此键 = 空快照（老行为）。
- `meta.common_config_enabled`：Common Config 三态开关（`Some(true)`=叠加 / `Some(false)`=不叠加 / 缺省=跟随全局默认，当前默认叠加）。全局 common config 片段本身存 `app_metadata` 键 `common_config_claude-code`（默认 `{"includeCoAuthoredBy":false}`），写 live 时 deep-merge 覆盖在 `meta.snapshot` 之上。
- **决策依据**：曾计划把 `settings_config` 从「端点引用」重定义为「全文快照」，重读调用图后放弃——`settings_config` 被 `resolve_direct_config`（Claude+Codex 共用）/tools-toggle/`reapply`/ccs 导入器同时消费为端点引用，重定义 blast radius 过大且违背本规范「providers 表按原样存 JSON」。改用 `meta` 子键 → 零 schema 迁移、Codex 路径零改动、既有测试零风险。`json_merge` / `claude_snapshot` 两个模块承载 deep_merge/deep_remove/strip/snapshot 读写算法。

## 启动期数据回填（区别于 schema 迁移）

schema 迁移（`MIGRATIONS`）只做 DDL；跨表的**数据**回填（如升级时把存量 `tool_takeover.enabled=1` 桥接成默认 `providers` 行）不放进迁移 SQL，而是独立的 DAO 函数在 `run_migrations` 之后、`AppState` 构造之前调用。

- **幂等**：回填必须可重复执行且第二次为 no-op。用确定性 id（如 `prov-backfill-<tool>`）而非随机 id；创建前依次检查「目标态是否已存在」（如 `get_current` 有值则跳过）与「确定性 id 是否已存在」（用户保留但改过的行不覆盖）。
- **不触发副作用链**：回填是纯 DB 写入，不调用会写外部文件/配置的服务函数（如 `tool_takeover::enable`）。回填时刻的一致性由前提保证，运行期不做主动校验/自动修复（避免"静默改 DB"的隐性行为）。
- **失败即 panic**：回填失败与迁移失败同等处理（`unwrap_or_else` + `panic!`），不让应用带着半迁移状态启动。
- 结果用 `#[derive(Default)]` 的 report struct 汇报（created / skipped_* 分类计数）并 `tracing::info!`,便于升级诊断。
