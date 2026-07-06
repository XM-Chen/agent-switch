# 修复 endpoint 凭据解密失败（aead::Error）与总览页残留错误

## Goal

排查并修复总览页显示的 `test-endpoint: 解密 API Key 失败: 解密失败: aead::Error` 历史错误,确认主密钥环境与凭据兼容性,决策修复策略。

## 问题现象

**观察时间**: v0.2.1 安装后

1. **总览页显示残留同步错误**:
   - "最近同步错误" 栏:`test-endpoint: 解密 API Key 失败: 解密失败: aead::Error`
   - "近期请求日志" 有 `502` 错误

2. **错误来源**:
   - `aead::Error` 是 AES-256-GCM 解密失败(crypto crate)
   - 记录在 `app_metadata` 表 `last_sync_error`(来自某次模型自动同步)
   - `test-endpoint` 的 `api_key_encrypted` 解密失败过

## 可能根因

1. **主密钥环境变化**: 跨版本(0.1.0→0.2.0/0.2.1) Keychain master key 变了,`test-endpoint` 的密文是旧 key 加密的,新 key 解不开。AAD 绑定 endpoint.id,密文挪到新环境解密失败是预期(security by design)。

2. **历史错误残留**: `last_sync_error` 是历史记录,不自动清除。即使 endpoint 后来删除/重新加密,错误仍显示。

3. **`test-endpoint` 来源未知**: 用户手动创建?还是测试残留?若不需要应删除。

## Requirements

1. **数据排查**:
   - query `endpoints` 表是否有 `name='test-endpoint'`?
   - 其 `api_key_encrypted` 是否非空?`created_at`?
   - `app_metadata` 的 `last_sync_error` 当前值?

2. **主密钥环境确认**:
   - 当前 Keychain master key 能否解密其他 endpoint 的凭据?
   - 能否用当前 key 解密 `test-endpoint` 的密文?

3. **修复策略决策**(排查后定):
   - 删除 `test-endpoint`(若不需要)
   - 清除 `last_sync_error`(手动 DELETE 或等下次成功同步覆盖)
   - 重新加密凭据(若 endpoint 需要保留但凭据不兼容)
   - 文档化跨版本凭据迁移限制

## Acceptance Criteria

- [ ] AC1: 排查清楚 `test-endpoint` 当前状态(是否存在、凭据是否可解密、来源)
- [ ] AC2: 确认主密钥环境变化与凭据不兼容的根因
- [ ] AC3: 执行修复策略(删除 endpoint / 清除错误 / 重新加密)
- [ ] AC4: 总览页不再显示 `test-endpoint` 的解密错误
- [ ] AC5: 若凭据不兼容是已知限制,记录进 spec 或发版文档

## Out of Scope

- 主密钥自动迁移(0.1.0→0.2.1 无迁移机制设计,本次不做)
- 凭据重加密工具(若需手动迁移,作为后续演进)

## Notes

先做数据排查(SQL + Keychain 日志),确认根因后再细化修复步骤。可能是轻量 bug(删 endpoint + 清错误),也可能需设计凭据迁移策略。

---

## 排查结论与处置（2026-07-06 已完成）

**实测数据**（`~/AppData/Roaming/com.agent-switch.app/agent-switch.db`）：
- `endpoints` 表 **0 行**——`test-endpoint` 早已被删除。
- `app_metadata.last_model_sync_error` = `test-endpoint: 解密 API Key 失败: 解密失败: aead::Error`，`last_model_sync_at` = **2026-06-28**（很久前）。

**真实根因**（非主密钥环境问题）：
- `last_model_sync_error` 是历史错误快照。`model_sync.rs:109-117` 只在**下次同步成功时**才清空它（`report.errors.is_empty()` → set ""）。
- `test-endpoint` 于 6-28 同步失败后被删除，但此后未再触发成功同步（自动刷新关闭或长期未启动），旧错误一直残留，总览页无脑显示。
- 502 请求日志同理：是 6-28 代理转发到 test-endpoint 解密失败留下的历史 `request_logs` 记录。

**处置**：
- ✅ 立即修复：`UPDATE app_metadata SET value='' WHERE key='last_model_sync_error'`——总览页"最近同步错误"已消失（用户确认）。
- **502 历史日志：保留**（合理的历史记录，`request_logs` 有 `MAX_LOG_ROWS` 自动清理机制，新请求进来会自然挤出；清掉反而丢信息）。
- **代码层隐患：本期不修**（边缘 case——仅"删光所有 endpoint 且长期不再同步"才残留；正常使用不踩）。如需根治，后续可加"endpoint 删除时清除其相关 sync error"或"总览显示时过滤已不存在 endpoint 的错误"，另开任务。

**AC 落定**：AC1（test-endpoint 已删、endpoints 空）✅ / AC2（根因是错误快照未清除，非凭据不兼容）✅ / AC3（清除 last_model_sync_error）✅ / AC4（总览页错误消失，用户确认）✅ / AC5（非凭据不兼容，无需记 spec；根因与处置已记本 PRD）✅。
