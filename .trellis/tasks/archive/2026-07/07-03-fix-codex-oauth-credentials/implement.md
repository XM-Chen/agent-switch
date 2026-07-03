# Codex OAuth 登录链路与凭据修复 - Implement

## 执行顺序

### Step 1: 启动前确认

```bash
python ./.trellis/scripts/task.py current
```

确认活动任务为 `.trellis/tasks/07-03-fix-codex-oauth-credentials`。

### Step 2: 阅读当前实现

精读:

- `src-tauri/src/services/codex_oauth.rs` (handle_callback, cleanup_session, parse_jwt_fields, exchange_code_for_token, TokenResponse, account_id 处理)
- `src-tauri/src/http/api/auth.rs`
- `src-tauri/src/http/proxy/constants.rs` (确认 OAUTH_REFRESH_TIMEOUT_SECS 已存在)
- `src-tauri/src/services/translator/helpers.rs` (确认 map_role 是否已删除)

### Step 3: P2-21 状态码分类

`auth.rs` 的 `start_codex_login`: session-in-progress 错误返回 409, 其他内部错误返回 500。

若 `start_login` 返回 String,用已知前缀("已有 Codex OAuth 登录进行中")判 409,其余 500,注释说明。

### Step 4: P2-22 callback cleanup

`handle_callback` 所有早退路径前调用 `cleanup_session(&app_state, &callback_shutdown).await`:

- error 字段
- state mismatch
- missing code
- token exchange 失败
- 序列化失败
- DB 保存失败

### Step 5: P2-23 expires_at

用 `TokenResponse.expires_in` 计算 `expires_at` 写入 `CodexCredentials`。移除 `#[allow(dead_code)]`。

### Step 6: P2-24 account_id 稳定性

`parse_jwt_fields` 扩展读取 `sub`。account_id 优先级: chatgptAccountId > accountID > sub。都没有时 `handle_callback` 返回明确错误 `missing_account_id`,不创建随机 UUID 账号。

### Step 7: P3 JWT padding

`parse_jwt_fields` base64url 解码兼容 padded / no-pad。

### Step 8: P3 HTTP 超时

`exchange_code_for_token` 用带超时的 reqwest Client,复用 constants.rs 的 OAUTH_REFRESH_TIMEOUT_SECS / OAUTH_REFRESH_CONNECT_TIMEOUT_SECS。

### Step 9: P3 map_role 确认

确认 `helpers.rs` 中 `map_role` 已被 Batch1 删除。若仍有残留测试或引用,清理。如需要角色映射,在 spec 记录应在真实调用点重新设计。

### Step 10: 质量门

```bash
cd src-tauri
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib codex_oauth
cargo test --lib
```

本子任务应使 `cargo clippy --all-targets -- -D warnings` 通过 (修复 codex_oauth.rs:463 unused import connection)。

### Step 11: 自检

对照 PRD AC1~AC9。

## 风险

- 改 `start_login` 返回类型会扩散;优先用字符串前缀分类的最小方案。
- account_id 缺失返回错误后,用户无法登录某些特殊 JWT;需在 UI/日志明确提示,不静默失败。
- callback cleanup 要避免重复 shutdown 导致 panic;参考已有 cleanup_session 实现。

## 完成前检查

- [ ] start_codex_login 只有会话冲突返回 409
- [ ] callback 所有早退路径 cleanup
- [ ] expires_at 计算
- [ ] account_id 缺失不生成随机 UUID
- [ ] JWT padded 解析
- [ ] token exchange 超时
- [ ] map_role 残留清理
- [ ] codex_oauth.rs:463 unused import 修复
- [ ] cargo clippy --all-targets 通过
- [ ] cargo test --lib 通过
