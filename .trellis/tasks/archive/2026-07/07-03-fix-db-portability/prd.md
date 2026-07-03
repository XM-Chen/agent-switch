# DB 与 portability 导入导出修复

## Goal

修复数据库模型同步、导入导出完整性、日志清理性能、导入报告可解释性等 P2/P3 缺陷,确保模型刷新不会因空响应误禁用,full backup replace 真正覆盖完整状态,portable merge 不静默产生孤儿数据。

## Background

- 审计报告锚点:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md` §4 P2-14~19, §5 Database / Portability / model_sync
- 调研确认锚点:
  - `model_sync.rs:136-177` 空 `/v1/models` 仍调用 `mark_unavailable_except_in_tx`
  - `endpoint_models.rs:124-142` synced upsert conflict 不更新 `source`
  - `apply.rs:73-192` replace 未清 route_settings、未写 ui_settings
  - `portability/mod.rs:114-236` mode/kdf 未交叉校验
  - `request_logs.rs:224-241` `NOT IN` 清理旧日志性能差
  - `apply.rs:306-384` merge 孤儿模型/别名静默跳过或留空外键
  - `portability/mod.rs:269-296` DB 备份文件名时间戳仅替换 `:`

## Requirements

### P2 缺陷

- **P2-14** `model_sync.rs:142`:当上游 `/v1/models` 返回空 `data: []` 时,不得静默将所有旧 synced 模型标记不可用。应将空响应视为异常/可疑响应,跳过 `mark_unavailable_except_in_tx`,记录 `last_model_sync_error`,并保持已有模型可用性不变。
- **P2-15** `endpoint_models.rs:124`:`upsert_synced_in_tx` 在 `ON CONFLICT(endpoint_id, model_name) DO UPDATE` 时必须更新 `source='synced'` 或拒绝覆盖 `custom` 行,避免 custom/source 不一致。
- **P2-16** `portability/apply.rs:164`:full backup replace 导入必须先清空 `route_settings` 再按包内容恢复,不得保留包中不存在的 route rows。
- **P2-17** `portability/apply.rs:183`:full backup replace 导入必须写回 `Payload.ui_settings`,保持 app_metadata 白名单偏好 round-trip。
- **P2-19** `portability/mod.rs:135`:导入包必须交叉校验 `mode` 与 `kdf`:full_backup 必须使用主密钥/none,portable 必须使用 argon2id/password;畸形组合应在解密前给出明确错误。

### P3 / 质量项

- `model_sync.rs:87` `host_last` HashMap 只 insert 不读:删除或实现真正 host 分组限流。由于当前顺序刷新天然无同 host 并发,优先删除死变量并修正文案。
- `model_sync.rs:237` capabilities 硬编码 `chat/streaming/tool_calling`:改为解析上游模型元数据(若有),无字段时按协议/端点类型给出保守默认,不得把 embeddings/images/audio-only 模型误标为 chat。
- `request_logs.rs:229` `prune_old` O(N²) 反连接:改为按 oldest id 批量删除或阈值删除。
- `portability/apply.rs:308` merge 对孤儿 endpoint_model/alias 静默跳过:ImportReport 增加 warning/skipped 计数或错误列表,至少在返回报告中可见。
- `portability/mod.rs:281` DB 备份文件名时间戳应使用 Windows 安全格式(如 `YYYYMMDD-HHMMSSZ`),不得含 `:` 或不必要的小数秒点。

## Design

### 空模型响应处理

推荐策略:

```rust
let models = fetch_models_from_endpoint(...).await?;
if models.is_empty() {
    return Err("上游 /v1/models 返回空 data 数组，已保留既有模型列表".to_string());
}
```

这样 sync_one_endpoint 不进入事务写入,不会调用 mark unavailable;上层 do_sync_all 把该端点记入 failed/errors,并更新 `last_model_sync_error`。

### synced/custom 冲突策略

选项 A(推荐):同步模型与同名 custom 模型冲突时把 source 改回 `synced`。这符合 `(endpoint_id, model_name)` 唯一约束下"同一端点同名模型只能有一个真实来源"的当前数据模型。

选项 B:拒绝覆盖 custom,为 synced 生成不同 id 但会违反唯一约束,需要 schema 变更,不适合本修复。

采用 A:
```
ON CONFLICT ... DO UPDATE SET
  source='synced',
  ...
```

### ImportReport warnings

若当前 `ImportReport` 无 warnings 字段,可扩展:

```rust
pub struct ImportReport {
  ...,
  pub warnings: Vec<String>,
  pub skipped_endpoint_models: usize,
  pub skipped_model_aliases: usize,
}
```

前端可暂不展示,但 API 返回中可见。

## Acceptance Criteria

- [ ] AC1(P2-14):空 `/v1/models` 响应不会禁用旧 synced 模型,并返回/记录错误
- [ ] AC2(P2-15):sync upsert 冲突后 source 正确为 `synced`,单测覆盖 custom→synced 冲突
- [ ] AC3(P2-16):replace 导入会删除包外 route_settings 行
- [ ] AC4(P2-17):replace 导入写回 ui_settings,偏好 round-trip 单测覆盖
- [ ] AC5(P2-19):mode/kdf 畸形组合在解密前被明确拒绝
- [ ] AC6(P3):host_last 死代码删除;capabilities 不再全量硬编码同一集合
- [ ] AC7(P3):request_logs prune 改为线性或近线性删除策略
- [ ] AC8(P3):merge 孤儿模型/别名有 warnings/skipped 报告,不再静默
- [ ] AC9(P3):DB 备份文件名 Windows 安全
- [ ] AC10:`cargo test --lib` 中相关 DAO/portability/model_sync 测试通过
- [ ] AC11:`cargo check` 0 warning,`cargo clippy --all-targets -- -D warnings` 通过(本子任务范围内)

## Out of Scope

- app_data_dir CWD 回退(P2-20)由 `07-03-fix-fmt-spec-alignment` 处理
- Codex OAuth 登录链路(P2-21~24)由 `07-03-fix-codex-oauth-credentials` 处理
