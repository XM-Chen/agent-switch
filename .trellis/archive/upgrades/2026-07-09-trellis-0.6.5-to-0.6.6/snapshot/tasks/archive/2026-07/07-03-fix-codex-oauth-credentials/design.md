# Codex OAuth 登录链路与凭据修复 - Design

## 1. 范围

修改 Codex OAuth 登录回调、状态码映射、JWT 解析、HTTP 超时、account_id 稳定性,以及 `helpers.rs` 中 `map_role` 死代码的最终处理。不得改 proxy failover、translator wire format、DB portability 或前端页面。

## 2. 关键文件

- `src-tauri/src/services/codex_oauth.rs`
- `src-tauri/src/http/api/auth.rs`
- `src-tauri/src/services/translator/helpers.rs` (仅 map_role 残留处理)
- 可能涉及 `src-tauri/src/services/mod.rs` 或类型定义处的错误枚举

## 3. 错误状态码映射

`start_codex_login` 当前把所有错误映射为 409。改为区分:

- 已有登录进行中 → 409 CONFLICT
- 端口绑定失败 / OAuth metadata 获取失败 / 内部错误 → 500 INTERNAL_SERVER_ERROR (或 503)

最小实现: 用 typed enum 或对错误字符串前缀分类。若改动 `start_login` 返回类型过大,保留 String 返回但 `auth.rs` 里按已知前缀(如 "已有 Codex OAuth 登录进行中")判 409,其余 500,并在代码注释说明。

## 4. callback cleanup

`handle_callback` 中所有早退路径必须调用 `cleanup_session`:

- error 字段
- state mismatch
- missing code
- token exchange 失败
- 序列化失败
- DB 保存失败

确保 session 和 callback 1455 端口在任何早退都被释放。

## 5. expires_at 计算

初始登录时用 `TokenResponse.expires_in` 计算 `expires_at`:

```rust
let expires_at = token_resp.expires_in.and_then(|s| {
    (OffsetDateTime::now_utc() + time::Duration::seconds(s as i64))
        .format(&Iso8601::DEFAULT).ok()
});
```

写入 `CodexCredentials.expires_at`。

## 6. account_id 稳定性

JWT 解析优先级:

1. `chatgptAccountId`
2. `accountID`
3. `sub` (JWT subject)
4. 都没有 → 返回明确错误 `missing_account_id`,不创建随机 UUID 账号

`parse_jwt_fields` 扩展读取 `sub`,并返回诊断信息便于日志。

## 7. JWT padding

`parse_jwt_fields` 的 base64url 解码兼容 padded 和 no-pad:

- 先尝试 `URL_SAFE_NO_PAD`
- 失败再尝试 `URL_SAFE` (允许 padding)
- 或手动 strip `=` 后用 NO_PAD

保证含 `=` padding 的 JWT payload 能解析。

## 8. HTTP 超时

`exchange_code_for_token` 使用带超时的 reqwest Client:

```rust
Client::builder()
    .timeout(Duration::from_secs(constants::OAUTH_REFRESH_TIMEOUT_SECS))
    .connect_timeout(Duration::from_secs(constants::OAUTH_REFRESH_CONNECT_TIMEOUT_SECS))
    .build()
```

复用 Batch1 在 `constants.rs` 已新增的 OAuth 超时常量。

## 9. map_role 处理

Batch1 translator 子任务已删除 `map_role` 死代码。本任务确认 `helpers.rs` 中已无 `map_role`;若仍有残留(如测试)一并清理,并在 spec 记录:角色映射如未来需要应在真实调用点重新设计,而非恢复 stub。

## 10. 测试设计

- JWT padding 解析 (padded / no-pad)
- account_id 优先级 (chatgptAccountId > accountID > sub > missing)
- expires_at 计算
- start_login 状态码分类 (session-in-progress 409, 其他 500)
- callback 早退路径 cleanup (用状态检查或代码审查)

## 11. 非目标

- OAuth refresh 预检链路 (Batch1 proxy 子任务)
- app_data_dir CWD 回退 (fmt-spec 子任务)
- OpenAI 通用 OAuth / Anthropic / Google OAuth provider
