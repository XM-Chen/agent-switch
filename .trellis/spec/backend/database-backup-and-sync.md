# 数据库、备份与同步

## SQLite 边界

`Database` 以 `Mutex<rusqlite::Connection>` 共享连接（`src-tauri/src/database/mod.rs:71-77`），schema version 当前为 11（`database/mod.rs:49-51`）。

目录职责：

- `schema.rs`：建表、索引、逐版迁移；
- `migration.rs`：旧 JSON → SQLite；
- `dao/`：领域查询；
- `backup.rs`：SQL export/import、本地文件备份/恢复；
- `mod.rs`：连接、PRAGMA、启动迁移和清理（`database/mod.rs:91-159`）。

业务和 command 不直接锁连接执行散落 SQL；新增查询进入对应 DAO。

## 首期 schema 策略

Claude-only 裁剪首期保持 schema v11：

- `providers` 的 `(id, app_type)` 主键、其他表的 `app_type` 列和未使用 enable 列暂留；
- 新业务只创建/读取 Claude 数据；
- 不为表面整洁引入 drop column/table 或重写历史；
- 稳定后另建 schema compact 任务。

每个迁移必须：递增版本、向前测试、已有 DB 预迁移备份、未来版本拒绝降级写入。

## 本地备份

保留 ccs 的 SQLite 一致性备份：

- 定时/手动创建、保留数量、列出与恢复；
- 恢复前创建 safety backup；
- 文件名和目录身份改为 Agent Switch，但不改变一致性语义；
- 只操作 `~/.agent-switch/backups`，禁止读写 `~/.cc-switch/backups`。

## WebDAV / S3 同步

保留 v2 artifact：`manifest.json + db.sql + skills.zip`。`backup.rs` 导出 provider 的 `settings_config`（`src-tauri/src/database/backup.rs:47-59,703-714`），因此 SQL 可能包含 API token。

安全不变量：

- TLS 只保护传输，不等于客户端内容加密；
- 首次启用远端同步前明确披露 `db.sql` 未客户端加密、可能含 API token、远端管理员/凭据持有者可读，并持久化显式确认；
- 日志不得输出 SQL 内容、token 或远端凭据；
- remote root/manifest identity 改为 Agent Switch，与 CC Switch 隔离；
- Claude-only 裁剪后 artifact 不应新增非 Claude 数据；历史 v11 列存在不代表要导出幽灵业务行；
- restore 先验证 manifest/产品标识/schema，保留 local-only 表规则和安全备份。

## 数据身份迁移

目标数据根为 `~/.agent-switch`，数据库为 `agent-switch.db`。首次启动全新空库，不自动读取/迁移：

- `~/.cc-switch/cc-switch.db`；
- 旧 Agent Switch OS AppData DB。

这与导入现有 `~/.claude/settings.json` 不冲突：产品 DB 不迁移，但用户当前 Claude live 必须先保护性导入。

## 测试

- 内存 DB 覆盖 DAO/schema；
- 临时目录覆盖文件备份、恢复与错误路径；
- v10→v11 与未来版本拒绝；
- sync export 不含 local-only 表，import 后恢复本地表；
- remote root/manifest 不含 ccs 身份；
- 新 HOME 下 `~/.cc-switch` 诱饵不变；
- 风险确认未完成时 WebDAV/S3 不允许启用。
