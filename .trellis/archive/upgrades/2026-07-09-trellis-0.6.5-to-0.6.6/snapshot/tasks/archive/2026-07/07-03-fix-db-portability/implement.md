# DB 与 portability 导入导出修复 - Implement

## 执行顺序

### Step 1: 启动前确认

```bash
python ./.trellis/scripts/task.py current
```

确认活动任务为 `.trellis/tasks/07-03-fix-db-portability`。

### Step 2: 阅读当前实现

精读:

- `src-tauri/src/services/model_sync.rs`
- `src-tauri/src/db/dao/endpoint_models.rs`
- `src-tauri/src/db/dao/request_logs.rs`
- `src-tauri/src/services/portability/mod.rs`
- `src-tauri/src/services/portability/apply.rs`
- `src-tauri/src/services/portability/collect.rs`
- `src-tauri/src/services/portability/package.rs`

### Step 3: P2-14 空 models 响应

`sync_one_endpoint` 在 `models.is_empty()` 时返回 Err,不进入事务。

### Step 4: P2-15 source 冲突

`upsert_synced_in_tx` 的 `ON CONFLICT DO UPDATE` 增加 `source='synced'`。

### Step 5: P2-16 replace route_settings

`apply_replace` 先 `DELETE FROM route_settings` 再 insert/upsert。

### Step 6: P2-17 replace ui_settings

`apply_replace` 写回 `p.ui_settings` 到 app_metadata 白名单键。

### Step 7: P2-19 mode/kdf 校验

`import` 在 `open_payload` 前交叉校验 mode/kdf。

### Step 8: P3 项

- 删除 `host_last` 死代码
- capabilities 推断替代硬编码
- `prune_old` 改为 oldest id 批量删除
- merge orphan 增加 warnings/skipped
- backup filename Windows-safe

### Step 9: 质量门

```bash
cd src-tauri
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

不强制 cargo fmt。

### Step 10: 自检

对照 PRD AC1~AC11。

## 风险

- 删除 host_last 时注意不要破坏顺序刷新的注释与 host 分组意图。
- capabilities 推断要保守,宁可少标不可误标 chat。
- prune_old 改 SQL 时注意 SQLite LIMIT 表达式限制,可先查 count 再 delete。
