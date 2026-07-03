# DB 与 portability 导入导出修复 - Design

## 1. 范围

本任务修改模型同步、endpoint_models DAO、portability 导入导出、request_logs 清理性能。不得修改 Codex OAuth callback、proxy failover、translator 或前端页面。

## 2. 关键文件

- `src-tauri/src/services/model_sync.rs`
- `src-tauri/src/db/dao/endpoint_models.rs`
- `src-tauri/src/db/dao/request_logs.rs`
- `src-tauri/src/services/portability/mod.rs`
- `src-tauri/src/services/portability/apply.rs`
- `src-tauri/src/services/portability/collect.rs`
- `src-tauri/src/services/portability/package.rs`

## 3. 模型同步契约

### 3.1 空 `/v1/models` 响应

`data: []` 视为可疑/失败响应,不得进入 mark unavailable 阶段。

实现策略:

```rust
let models = fetch_models_from_endpoint(...).await?;
if models.is_empty() {
    return Err("上游 /v1/models 返回空 data 数组，已保留既有模型列表".to_string());
}
```

上层 sync report 将该端点标记 failed 并写 last_model_sync_error。

### 3.2 synced/custom source 冲突

当前唯一约束是 `(endpoint_id, model_name)`。因此同端点同名模型只能有一个 row。同步返回同名模型时,该 row 应成为 `source='synced'`。

`upsert_synced_in_tx` conflict update 必须设置:

```sql
source='synced'
```

### 3.3 capabilities 推断

不再对所有模型硬编码 `chat,streaming,tool_calling`。

优先级:

1. 上游 item 已含 `capabilities` 数组:直接使用并规范化为字符串数组
2. 根据模型 id/name 做保守推断:
   - 包含 `embedding` / `embed`: `embeddings`
   - 包含 `image` / `dall-e` / `gpt-image`: `images`
   - 包含 `tts` / `whisper` / `audio`: `audio`
   - 其他默认: `chat`;streaming/tool_calling 只对已知 chat 家族或协议支持时添加
3. 记录该推断是 best-effort,后续可扩展 provider-specific metadata

## 4. portability replace 契约

full_backup replace 必须是完整覆盖:

1. 清空 accounts/endpoints/endpoint_models/model_aliases/route_settings/tool_takeover/app_metadata 白名单键
2. 按包内容恢复
3. tool_takeover 仍强制 disabled,不自动写工具配置
4. ui_settings 必须写回 app_metadata 白名单键

`route_settings` 不再只 upsert,必须先 delete 再 insert/upsert。

## 5. mode/kdf 交叉校验

在 `open_payload` 前增加:

| mode | kdf | 是否允许 |
|------|-----|----------|
| full_backup | none/master-key | 允许 |
| portable | argon2id | 允许 |
| full_backup | argon2id | 拒绝 |
| portable | none/master-key | 拒绝 |

错误信息必须指出 mode/kdf 不匹配,而不是笼统解密失败。

## 6. ImportReport warnings

为 merge 的 skipped/orphan 增加可见报告:

```rust
warnings: Vec<String>
skipped_endpoint_models: usize
skipped_model_aliases: usize
```

孤儿 endpoint_model 不再静默 continue,必须增加 warning。孤儿 alias 如果 target endpoint 无法 remap,应跳过并 warning,避免写入 NULL target 或保留旧外键。

## 7. request_logs prune

改为直接删除最旧 N 行:

```sql
DELETE FROM request_logs
WHERE id IN (
  SELECT id FROM request_logs
  ORDER BY created_at ASC, id ASC
  LIMIT (
    SELECT MAX(COUNT(*) - ?1, 0) FROM request_logs
  )
)
```

如果 SQLite 不支持该表达式,可先查询总数再 delete limit。目标是避免 `NOT IN` 反连接。

## 8. DB backup filename

使用 Windows-safe 时间戳:

```text
YYYYMMDD-HHMMSSZ
```

不得含 `:`、`/`、`\`、小数秒点或空格。

## 9. 测试设计

- 空 models 响应不禁用旧模型
- upsert custom→synced 后 source 为 synced
- replace 删除包外 route_settings
- replace 恢复 ui_settings
- mode/kdf mismatch 在解密前报错
- merge orphan models/aliases 返回 warnings/skipped
- prune_old 删除最旧行且保留最新 max_rows
- backup filename 不含 Windows 非安全字符
