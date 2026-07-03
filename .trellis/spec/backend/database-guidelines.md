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

## request_logs

- 请求日志只保存摘要，不保存 prompt/messages/完整 headers/API key/token。
- prune 应按 overflow 删除最旧行，避免 `NOT IN (SELECT ...)` 反连接造成大表 O(N²) 风险。
