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
- `meta.snapshot.env`：Claude Code 行为开关（非连接层）承载点。前端结构化编辑 DEFAULT 模型、`API_TIMEOUT_MS`、`CLAUDE_CODE_*`、Bedrock/AWS env 后，必须写回这里；后端只透传 `providers.meta`，落 live 必须通过 `POST /api/providers/{id}/switch` 重切来复用快照层，禁止新增直接写 `settings.json` 的 env 专用路径。`API_TIMEOUT_MS` 空串=删键，非空必须为正整数毫秒；Bedrock 明文键（`CLAUDE_CODE_USE_BEDROCK`/`AWS_REGION`/`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`）按用户决策存 DB + live 明文，连接 token 仍不得存入 snapshot。
- `meta.common_config_enabled`：Common Config 三态开关（`Some(true)`=叠加 / `Some(false)`=不叠加 / 缺省=跟随全局默认，当前默认叠加）。全局 common config 片段本身存 `app_metadata` 键 `common_config_claude-code`（默认 `{"includeCoAuthoredBy":false}`），写 live 时 deep-merge 覆盖在 `meta.snapshot` 之上。
- **决策依据**：曾计划把 `settings_config` 从「端点引用」重定义为「全文快照」，重读调用图后放弃——`settings_config` 被 `resolve_direct_config`（Claude+Codex 共用）/tools-toggle/`reapply`/ccs 导入器同时消费为端点引用，重定义 blast radius 过大且违背本规范「providers 表按原样存 JSON」。改用 `meta` 子键 → 零 schema 迁移、Codex 路径零改动、既有测试零风险。`json_merge` / `claude_snapshot` 两个模块承载 deep_merge/deep_remove/strip/snapshot 读写算法。

#### Code-spec: Claude Code env 行为开关（`meta.snapshot.env`）

##### 1. Scope / Trigger
- Trigger：新增/维护 Claude Code 行为 env（模型三档、超时、Bedrock、`CLAUDE_CODE_*`）时，属于跨层 `providers.meta` 合约 + live `settings.json` 快照层。

##### 2. Signatures
- DB：`providers.meta` JSON string；`meta.snapshot.env` 为 JSON object。
- HTTP：`PUT /api/providers/{id}` 的 `meta?: Value` 可更新整份 meta；`POST /api/providers/{id}/switch` 负责把当前 provider 的 `meta.snapshot.env` 写入 live。
- Frontend helper：`parseClaudeEnv(meta) -> ClaudeEnvSwitches`；`serializeClaudeEnv(meta, switches) -> meta`；`validateApiTimeoutMs(value) -> string | null`。

##### 3. Contracts
- 连接层 env：`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` 不由前端结构化编辑器暴露，不应由 env-switch 写入；切换 backfill 会剥离。
- 行为层 env：`ANTHROPIC_DEFAULT_{HAIKU,SONNET,OPUS}_MODEL(_NAME)`、`ANTHROPIC_MODEL`、`API_TIMEOUT_MS`、`CLAUDE_CODE_*`、Bedrock/AWS 键属于 `meta.snapshot.env`。
- 空字符串表示删除该 env 键；非空值写入时 trim。
- 应用到 live：保存 provider 后调用 switch 同一 id（`prev=target` 重切），不得新增独立写 live env API。

##### 4. Validation & Error Matrix
- `API_TIMEOUT_MS === ''` -> 允许，序列化删除键。
- `API_TIMEOUT_MS` 非空且不是正整数 -> 前端拦截并显示错误，不调用 update。
- `meta` 非对象/缺失 -> helper 按空 meta 处理，不 panic。
- direct 连接凭证不可解密 -> switch 返回 503，不把 env-only 改动静默落 live。

##### 5. Good/Base/Bad Cases
- Good：用户编辑 `API_TIMEOUT_MS=60000` → `PUT providers.meta.snapshot.env.API_TIMEOUT_MS` → 对当前 provider 点「应用到 live」→ switch 写入 `settings.json.env.API_TIMEOUT_MS`。
- Base：非当前 provider 保存 env → 只更新 DB；下次切换到该 provider 自然生效。
- Bad：前端直接写 `settings.json` 或新增 `/api/providers/{id}/env/apply` 绕过 `switch_claude`，会丢失回填保护/连接层剥离/Common Config 三层。

##### 6. Tests Required
- Helper：parse/serialize round-trip、空值删键、`API_TIMEOUT_MS` 正整数校验、连接键不被结构化 helper 写入。
- Backend：switch 回归覆盖 `meta.snapshot.env` 往返与连接 token 不落库（可在 `tool_takeover` 快照测试中断言）。
- Frontend：build/test 必须覆盖 helper；页面按钮 pending guard 避免重复 switch。

##### 7. Wrong vs Correct

Wrong:
```ts
await fetch('/api/providers/1/env/apply', { body: JSON.stringify(env) }); // 绕过 switch
```

Correct:
```ts
await providersApi.update(id, { meta: serializeClaudeEnv(oldMeta, switches) });
await providersApi.switch(id); // 复用 switch_claude(prev=target)
```

## Prompts 管理数据模型（Claude Code `CLAUDE.md` 单激活）

`prompts` 表（迁移 v10 `create_prompts`）是 Claude Code `~/.claude/CLAUDE.md` 的可管理提示词清单。它是**全局 Prompts 清单**，不挂在 provider 切换上；任一时刻至多一份 `enabled_claude=1` 并投影到 live `CLAUDE.md`。

### 1. Scope / Trigger
- Trigger：新增/维护 Claude Code Prompts 管理、`CLAUDE.md` 投影、反向导入或首次启动导入时，属于 DB schema + DAO + live 文件副作用合约。

### 2. Signatures
- DB schema：`prompts(id TEXT PRIMARY KEY, name TEXT NOT NULL, content TEXT NOT NULL, description TEXT, enabled_claude INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)`。
- DAO：`PromptRow` / `NewPrompt` / `PromptUpdate`；`list/get/create/update/delete/get_enabled_claude/set_enabled_exclusive`。
- Service：`enable_prompt(db,id)`、`disable_prompt(db,id)`、`delete_prompt(db,id)`、`import_from_claude(db)`、`import_on_first_launch(db)`、`get_status(db)`。

### 3. Contracts
- `content` 明文存储，写 live 时原样投影到 `~/.claude/CLAUDE.md`；这不同于 endpoints/accounts 的 API key 加密边界。
- 单激活必须由 DAO 事务保证：启用 B = 同事务清空所有 `enabled_claude` 再置 B=1；目标不存在必须回滚并报错。
- 启用前必须做回填保护：若已有启用项，将当前 live `CLAUDE.md` 回填进旧启用项；若无启用项且 live 内容不在 DB 中，创建 backup prompt；重复内容跳过。
- 未安装跳过：`CLAUDE.md` 与其父目录 `~/.claude/` 均不存在时，同步 no-op；父目录必须从传入 path 的 parent 推导，测试不得查真实 home。
- 启动期 `import_on_first_launch` 是数据导入，不是 schema migration；DB 非空时必须 no-op。

### 4. Validation & Error Matrix
- enable 不存在 id -> 400（HTTP 层映射），事务回滚。
- delete 已启用 prompt -> 400；必须先 disable 或启用其它 prompt。
- live `CLAUDE.md` 缺失/空内容导入 -> imported=0，不报错。
- `~/.claude` 未安装 -> enable/disable 不创建目录或文件，DB 激活态仍可更新。
- IO/原子写失败 -> 500，错误消息可诊断。

### 5. Good/Base/Bad Cases
- Good：启用 B 时先保存 live 手改到 A，再单激活 B，最后写 B.content 到 `CLAUDE.md`。
- Base：首次启动 DB 空且 live 非空 -> 导入一份 disabled prompt；后续启动 DB 非空 -> 0 导入。
- Bad：直接 `UPDATE prompts SET enabled_claude=1 WHERE id=?`，会造成多激活并绕过回填保护。

### 6. Tests Required
- DAO：CRUD round-trip、`get_enabled_claude` 至多一份、`set_enabled_exclusive` 原子性和 missing id 回滚。
- Service：启用投影、回填旧激活项、无激活 backup、重复 backup 跳过、禁用最后一份清空 live、删除保护、反向导入、首次导入幂等、未安装 enable/disable no-op。
- HTTP/UI：删除激活项错误映射、导入按钮刷新列表/status、Prompt 内容 textarea 往返。

### 7. Wrong vs Correct

Wrong:
```rust
// 绕过事务和回填保护
conn.execute("UPDATE prompts SET enabled_claude = 1 WHERE id = ?", [&id])?;
```

Correct:
```rust
// service 先 backfill live，再调用 DAO 事务，再按 should_sync 原子投影 live
prompts::claude::enable_prompt(&db, &id)?;
```

## 启动期数据回填（区别于 schema 迁移）

schema 迁移（`MIGRATIONS`）只做 DDL；跨表的**数据**回填（如升级时把存量 `tool_takeover.enabled=1` 桥接成默认 `providers` 行）不放进迁移 SQL，而是独立的 DAO 函数在 `run_migrations` 之后、`AppState` 构造之前调用。

- **幂等**：回填必须可重复执行且第二次为 no-op。用确定性 id（如 `prov-backfill-<tool>`）而非随机 id；创建前依次检查「目标态是否已存在」（如 `get_current` 有值则跳过）与「确定性 id 是否已存在」（用户保留但改过的行不覆盖）。
- **不触发副作用链**：回填是纯 DB 写入，不调用会写外部文件/配置的服务函数（如 `tool_takeover::enable`）。回填时刻的一致性由前提保证，运行期不做主动校验/自动修复（避免"静默改 DB"的隐性行为）。
- **失败即 panic**：回填失败与迁移失败同等处理（`unwrap_or_else` + `panic!`），不让应用带着半迁移状态启动。
- 结果用 `#[derive(Default)]` 的 report struct 汇报（created / skipped_* 分类计数）并 `tracing::info!`,便于升级诊断。
