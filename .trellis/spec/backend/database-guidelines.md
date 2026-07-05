# 数据库与模型同步规范

## SQLite 连接

- `rusqlite::Connection` 通过 `Arc<Mutex<Connection>>` 共享。
- 不要在持有 DB mutex 时等待外部网络 I/O。
- 多表一致性写入必须使用事务。

## DAO 约定

- Handler/service 调用 DAO，不直接拼散落 SQL。
- DAO 返回 `Result<_, String>` 时错误消息必须可诊断。
- 模型能力字段是 JSON 字符串数组；能力过滤必须做 JSON token 级匹配，禁止 `LIKE '%cap%'` 子串匹配。

## 模型同步

- `/v1/models` 返回空 `data: []` 视为异常/可疑响应，不得批量禁用既有 synced 模型。
- synced upsert 与同端点同名模型冲突时，不得破坏 custom 模型；若当前 schema 只能有一行，则必须明确 source 语义并配测试。
- capabilities 优先使用上游字段；缺失时保守推断，不能把 embeddings/images/audio-only 模型误标为 chat/tool_calling。

## 启动期数据回填（区别于 schema 迁移）

schema 迁移（`MIGRATIONS`）只做 DDL；跨表的**数据**回填（如升级时把存量 `tool_takeover.enabled=1` 桥接成默认 `providers` 行）不放进迁移 SQL，而是独立的 DAO 函数在 `run_migrations` 之后、`AppState` 构造之前调用。

- **幂等**：回填必须可重复执行且第二次为 no-op。用确定性 id（如 `prov-backfill-<tool>`）而非随机 id；创建前依次检查「目标态是否已存在」（如 `get_current` 有值则跳过）与「确定性 id 是否已存在」（用户保留但改过的行不覆盖）。
- **不触发副作用链**：回填是纯 DB 写入，不调用会写外部文件/配置的服务函数（如 `tool_takeover::enable`）。回填时刻的一致性由前提保证，运行期不做主动校验/自动修复（避免"静默改 DB"的隐性行为）。
- **失败即 panic**：回填失败与迁移失败同等处理（`unwrap_or_else` + `panic!`），不让应用带着半迁移状态启动。
- 结果用 `#[derive(Default)]` 的 report struct 汇报（created / skipped_* 分类计数）并 `tracing::info!`,便于升级诊断。

## request_logs

- 请求日志只保存摘要，不保存 prompt/messages/完整 headers/API key/token。
- prune 应按 overflow 删除最旧行，避免 `NOT IN (SELECT ...)` 反连接造成大表 O(N²) 风险。
