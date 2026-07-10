# 技术设计 — 修复重复 OAuth 登录同一 ChatGPT 账号 PK 冲突丢 token(P1-3)

## 边界

仅改 `src-tauri/src/services/codex_oauth.rs` 的 `handle_callback`(`184-340`,重点是 `269-315` 的写库分支)。`db/dao/accounts.rs` 已有 `get`/`create`/`update` 三个 helper,直接复用,**不新增 DAO helper**。不动 `http/api/accounts.rs`(通用 create 端点对 oauth_codex 的拒绝逻辑保留)、不动 `oauth_refresh.rs`、不动 portability apply。

## 当前数据流(handle_callback,184-340)

1. 校验 `error`(`195-204`):有则 cleanup + 400。
2. 校验 `state`(`207-215`):不匹配直接 return(P2-22,本任务不修)。
3. 取 `code`(`217-228`):缺失 cleanup + 400。
4. `exchange_code_for_token`(`231`)→ `TokenResponse { access_token, refresh_token, id_token, expires_in }`。
5. 解析 JWT(`234-238`)→ `(account_id, email, plan_type)`。
6. 构造 `CodexCredentials`(`240-248`),`expires_at: None`(P2-23,本任务不修)。
7. 序列化凭据 JSON(`251-263`)。
8. `account_id_str = account_id.unwrap_or_else(random_uuid)`(`265-267`,P2-24,本任务不修)。
9. `crypto.encrypt(&json, account_id_str.as_bytes())`(`271`)→ `encrypted` BLOB。
10. 构造 `NewAccount { id: account_id_str, ... }`(`285-293`)。
11. **`accounts::create(&app_state.db, new_account)`(`294`)→ 纯 INSERT,二次登录 PK 冲突返回 Err**。
12. Err 分支(`294-303`):cleanup + 500 `db_save_failed`,新 token 丢弃。
13. 成功(`318-327`):cleanup + 200 `{status, email, plan_type}`。

## 修复设计(upsert:已存在 update / 不存在 create)

在步骤 11 处替换「无条件 create」为「先 get 判存,再 update 或 create」:

```
// 步骤 11 替换为:
let existing = accounts::get(&app_state.db, &account_id_str)
    .map_err(|e| { /* cleanup + 500 db_query_failed */ })?;

match existing {
    Some(_) => {
        // 重登录:覆盖凭据 + 名称,不动 account_type/platform/status/priority
        let upd = accounts::AccountUpdate {
            name: Some(email.clone().unwrap_or_else(|| "Codex 账号".to_string())),
            credentials_encrypted: Some(Some(encrypted)),
            ..Default::default()
        };
        if let Err(e) = accounts::update(&app_state.db, &account_id_str, upd) {
            cleanup_session(...);
            return (500, json!({"error":"db_save_failed","message":e}));
        }
    }
    None => {
        // 首次登录:create(维持现有 NewAccount 构造,285-293)
        let new_account = accounts::NewAccount { /* 现有字段 */ };
        if let Err(e) = accounts::create(&app_state.db, new_account) {
            cleanup_session(...);
            return (500, json!({"error":"db_save_failed","message":e}));
        }
    }
}
// 两条路径成功后统一走 318-327 的 cleanup + 200 响应
```

### 字段覆盖决策

| 列 | update 路径是否覆盖 | 理由 |
|----|---------------------|------|
| `credentials_encrypted` | ✅ 覆盖为新 BLOB | 核心:重登录的新 token 必须落库 |
| `name` | ✅ 覆盖为 email/"Codex 账号" | email 可能在重登录时变更(同账号换邮箱),与 JWT 一致 |
| `account_type` | ❌ 不动 | 首次 create 已写 `oauth_codex`,重登录不变 |
| `platform` | ❌ 不动 | 同上 `openai_codex` |
| `status` | ❌ 不动 | 不在 `AccountUpdate` 默认更新;重登录不改活跃态(若曾 `disabled` 由用户手动改,不在此处重置) |
| `priority` | ❌ 不动 | 用户可能已手动调整优先级,重登录不应重置 |
| `extra_json` | ❌ 不动 | 当前 NewAccount.extra_json 恒 None,无值可更新 |
| `last_login_at` | ❌ 不动 | 当前 OAuth 登录不写该字段(P2 范围),不引入新行为;保留为 P2-23 同族后续统一处理 |

> 仅覆盖 `credentials_encrypted` + `name`,其余维持原值,最小化副作用。

### account_id 作 AAD 的一致性

`crypto.encrypt(&json, account_id_str.as_bytes())` 以 account_id 作 AAD。重登录 update 路径中 account_id 不变(同一 `chatgptAccountId`),AAD 一致,`oauth_refresh` 后续 `crypto.decrypt(..., account_id.as_bytes())` 仍可正确解密。**无需重加密或改 AAD**。

## 兼容性

- **首次登录 create 路径**:`NewAccount` 构造(285-293)与 `accounts::create` 调用完全保留,行为不变。
- **oauth_refresh 路径**:走 `accounts::update`(只更新 `credentials_encrypted`/`last_error` 等),本任务不改动其调用,不受影响。
- **portability apply 路径**:走自己的事务内 INSERT/UPDATE,不经过 `handle_callback`,不受影响。
- **API handler `http/api/accounts.rs`**:对 `oauth_codex` 的 create 拒绝(95-100)保留——用户仍只能通过 `/api/auth/codex/login` 创建/更新 Codex 账号,符合契约。
- **P2-24 回退 UUID 场景**:account_id 缺失时 `account_id_str` 为随机 UUID,每次登录不同 → `accounts::get` 恒返回 None → 走 create 路径(不冲突)。本修复不改变该行为,P2-24 仍待其独立任务解决,但不会引入新回归。

## 测试设计

在 `codex_oauth.rs` 的 `#[cfg(test)]` mod(若不存在则新增)加测试。因 `handle_callback` 依赖 `AppState`(crypto、db、codex_oauth service)且启动回调服务较重,**优先对写库逻辑抽取可测 helper 后单测**;若重构成本高则用内存 SQLite + 真 crypto 构造 AppState 跑集成测试。

### 可测 helper 抽取(推荐)

把步骤 9-12 的「加密 → upsert 写库」抽成纯函数,降低测试门槛:

```rust
/// 加密凭据并 upsert 写库(已存在则 update,不存在则 create)。
/// 返回 Ok(()) 或 Err(error_msg)。
fn upsert_codex_account(
    app_state: &Arc<AppState>,
    account_id_str: &str,
    name: &str,
    credentials_json: &[u8],
) -> Result<(), String> {
    let crypto = app_state.crypto.as_ref().ok_or("加密服务不可用")?;
    let encrypted = crypto.encrypt(credentials_json, account_id_str.as_bytes())?;
    match accounts::get(&app_state.db, account_id_str)? {
        Some(_) => {
            let upd = accounts::AccountUpdate {
                name: Some(name.to_string()),
                credentials_encrypted: Some(Some(encrypted)),
                ..Default::default()
            };
            accounts::update(&app_state.db, account_id_str, upd)
        }
        None => {
            let new = accounts::NewAccount {
                id: account_id_str.to_string(),
                name: name.to_string(),
                account_type: "oauth_codex".to_string(),
                platform: "openai_codex".to_string(),
                credentials_encrypted: Some(encrypted),
                extra_json: None,
                priority: 0,
            };
            accounts::create(&app_state.db, new).map(|_| ())
        }
    }
}
```

`handle_callback` 步骤 9-12 改为调用此 helper,Err 分支统一 cleanup + 500。

### 测试用例

1. `upsert_codex_account_first_login_creates`:空 DB,`account_id="acc-A"`,调用 helper,断言 `accounts::get("acc-A")` 返回 Some、`account_type=="oauth_codex"`、`platform=="openai_codex"`、`has_credentials==true`。
2. `upsert_codex_account_relogin_updates_credentials`:先 create 一行 `acc-A`(旧 BLOB + name="old@email.com"),再用**不同** credentials_json 调 helper(新 BLOB + name="new@email.com"),断言:返回 Ok;`accounts::get("acc-A")` 仍只有一行(PK 未冲突);`credentials_encrypted` 等于新 BLOB(≠ 旧);`name=="new@email.com"`;`account_type`/`platform`/`priority` 保持原值。
3. `upsert_codex_account_crypto_unavailable_returns_error`:`app_state.crypto=None`,断言返回 Err(503 路径在 handler 层,helper 层返回错误串)。
4. (可选,handler 层集成)构造最小 `AppState`(内存 SQLite + 真 crypto + CodexOAuthService),模拟二次回调验证返回 200 + DB 行更新。若 handler 集成测试搭建成本过高,以 helper 单测覆盖即可,handler 层靠 code review 保证接线。

## 风险/回滚

- **风险 1**:`accounts::get` 与后续 `accounts::update`/`create` 之间存在 TOCTOU 窗口(并发两次 OAuth 回调同账号)。当前 `CodexOAuthService.session` 用 `tokio::sync::Mutex` 保证同一时刻只有一个登录会话(`start_login:68-70`),且回调服务单实例,实际并发概率为零。**不做额外加锁**,保留既有 session 互斥即可。若未来允许多会话,需在此处加 DB 级 `INSERT ... ON CONFLICT DO UPDATE` 替代 get-then-write。
- **风险 2**:update 路径若 `accounts::get` 返回 Some 但 `accounts::update` 影响行数 0(理论上 account_id 不变不会发生),会静默不更新。当前 `accounts::update` 不返回影响行数(只返 `Ok(())`),无法检测。**接受该风险**(account_id 在 get 与 update 间不变,UPDATE WHERE id=? 必命中)。
- **风险 3**:抽取 helper 改变了 `handle_callback` 结构,若 helper 签名设计不当会破坏 `cleanup_session` 时机。约定:`handle_callback` 仍负责 cleanup,helper 只返回 Result,cleanup 在调用方 Err 分支执行(与现有 create 调用模式一致)。
- **回滚点**:单文件 `codex_oauth.rs` 改动 + 可能新增 `#[cfg(test)]` mod;`git revert` 单 commit 即可回滚。DAO 层零改动。
