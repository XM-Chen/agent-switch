# Codex OAuth 登录链路与凭据修复

## Goal

修复 Codex OAuth 登录链路的错误状态码、回调清理、token 过期时间、账号身份稳定性、JWT padding 兼容、HTTP 超时等 P2/P3 缺陷,确保重复登录/异常回调不会卡死或产生重复账号。

## Background

- 审计报告锚点:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md` §4 P2-21~24, §5 Proxy/Stream/Failover/codex_oauth P3
- 调研确认:
  - `http/api/auth.rs:28-41` 所有 start_login error 均映射 409
  - `codex_oauth.rs:206-227` state mismatch/missing code 未 cleanup
  - `codex_oauth.rs:232-248` TokenResponse.expires_in 标记 dead_code,credentials.expires_at=None
  - `codex_oauth.rs:265-267` account_id 缺失回退随机 UUID
  - `codex_oauth.rs:412` JWT 使用 `URL_SAFE_NO_PAD`,含 `=` padding 时解析失败
  - `codex_oauth.rs:377` `exchange_code_for_token` 使用 `Client::new()` 无超时
  - `model_sync.rs:191` `fetch_models_from_endpoint` 无超时(由 db/portability 子任务或本任务协同处理)
  - `helpers.rs:13` `map_role` 零生产调用、文档与实现矛盾

## Requirements

### P2 缺陷

- **P2-21** `http/api/auth.rs:39`:只有"已有登录进行中"类错误返回 409;端口绑定失败、token endpoint 配置/网络、内部错误应返回 500/503/502 等更准确状态。需要把 `start_login` 错误改为 typed error 或至少字符串分类。
- **P2-22** `codex_oauth.rs:207`:OAuth callback `state` 不匹配、missing code 等早退路径必须调用 `cleanup_session`,释放 session 和 callback 1455 端口。
- **P2-23** `codex_oauth.rs:243`:初始登录必须使用 `expires_in` 计算 `expires_at`,写入 `CodexCredentials`。
- **P2-24** `codex_oauth.rs:265`:account_id 缺失时不得回退随机 UUID。应优先解析 `chatgptAccountId`/`accountID`;若仍缺失,使用稳定 fallback(如 email hash/subject claim)或拒绝保存并提示 `missing_account_id`。

### P3 / 质量项

- `codex_oauth.rs:412`:JWT payload 解码兼容 padded 和 no-pad base64url;不因 `=` padding 返回空字段。
- `codex_oauth.rs:363/377`:`exchange_code_for_token` 使用带超时的 reqwest Client。
- `model_sync.rs:191`:fetch_models_from_endpoint 无超时由 db/portability 子任务处理;若本任务触及共享 HTTP client,一起修。
- `helpers.rs:13` map_role 零生产调用且文档矛盾:删除该死代码,或实现并接入真实 translator。因 translator 子任务也处理 helpers 死代码,本任务只记录:若 map_role 仍存在,必须修正文档/实现并补测试;若已被 translator 子任务删除,本任务视为完成。
- `codex_oauth.rs:398` parse_jwt_fields 错误时应有 tracing::warn,便于诊断。

## Design

### Typed error

推荐新增:

```rust
pub enum CodexOAuthStartError {
    LoginInProgress,
    CallbackBindFailed(String),
    Internal(String),
}
```

API 映射:

| Error | HTTP |
|-------|------|
| LoginInProgress | 409 |
| CallbackBindFailed | 503 或 500 |
| Internal | 500 |

若改动过大,最小方案:保留 String,但对固定文本 `已有 Codex OAuth 登录进行中` 返回 409,其他返回 500。

### callback cleanup 早退

所有 `return BAD_REQUEST` 之前统一调用:

```rust
cleanup_session(&app_state, &callback_shutdown).await;
```

避免 session 和 port 残留。

### expires_at

```rust
let expires_at = token_resp.expires_in.map(|s| {
  (OffsetDateTime::now_utc() + time::Duration::seconds(s as i64))
    .format(&Iso8601::DEFAULT)
    .unwrap_or_default()
}).filter(|s| !s.is_empty());
```

### account_id fallback

优先级:
1. `chatgptAccountId`
2. `accountID`
3. `sub` claim(如果 JWT 有)
4. `email` 的稳定 hash(不暴露明文)或拒绝保存

推荐:优先扩展 JWT 解析读取 `sub`;若 account_id/sub 都没有,返回 `missing_account_id` 错误,不创建随机账号。email hash 可能把可变邮箱绑定为账号 id,不如明确失败安全。

## Acceptance Criteria

- [ ] AC1(P2-21):start_codex_login 只有会话冲突返回 409,其他错误不返回 409
- [ ] AC2(P2-22):state mismatch 与 missing code 都清理 session/callback shutdown
- [ ] AC3(P2-23):初始登录凭据包含 expires_at,单测或代码验证
- [ ] AC4(P2-24):account_id 缺失不会生成随机 UUID;可解析 sub 或返回明确错误
- [ ] AC5(P3):JWT padded/no-pad base64url 均能解析
- [ ] AC6(P3):token exchange HTTP client 有超时
- [ ] AC7(P3):map_role 死代码被删除或文档/实现/测试一致
- [ ] AC8:`cargo test --lib codex_oauth` 或相关单测通过
- [ ] AC9:`cargo check` 0 warning,`cargo clippy --all-targets -- -D warnings` 通过(本子任务范围内)

## Out of Scope

- OAuth refresh 预检链路(`oauth_refresh.rs`)由 `07-03-fix-proxy-oauth-failover` 处理
- app_data_dir CWD 回退(P2-20)由 `07-03-fix-fmt-spec-alignment` 处理
- OpenAI 通用 OAuth 或 Anthropic/Google OAuth provider
