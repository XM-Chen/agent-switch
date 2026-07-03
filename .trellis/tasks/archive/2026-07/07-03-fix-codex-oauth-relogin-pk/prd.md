# 修复重复 OAuth 登录同一 ChatGPT 账号 PK 冲突丢 token(P1-3)

> 父任务:`07-03-fix-audit-p1-defects`。审计来源:`codebase-audit` 报告 §3 P1-3。

## Goal

修复 `handle_callback` 在重复 OAuth 登录同一 ChatGPT 账号时,因 `accounts::create` 纯 INSERT 触发 `UNIQUE constraint failed: accounts.id` 而返回 500、丢弃新换发 token 的缺陷;改为「已存在则 update、不存在则 create」的 upsert 语义,使重新登录能正确更新凭据。

## Background(代码事实)

审计报告 P1-3(`src-tauri/src/services/codex_oauth.rs:294`)描述的数据流经代码复核确认成立:

1. `handle_callback`(`codex_oauth.rs:184-340`)在交换 token 成功后,解析 id_token JWT 取 `chatgptAccountId`(`parse_jwt_fields`,393-424,同一账号稳定 id)。
2. `account_id_str` 复用该 id(`codex_oauth.rs:265-267`);仅当 id 缺失时回退随机 UUID(P2-24,本任务不修)。
3. `crypto.encrypt(&json, account_id_str.as_bytes())`(`codex_oauth.rs:271`)用 account_id 作 AAD 加密凭据 BLOB。
4. 构造 `accounts::NewAccount { id: account_id_str, ... }`(`codex_oauth.rs:285-293`),调用 `accounts::create(&app_state.db, new_account)`(`codex_oauth.rs:294`)。
5. `accounts::create`(`dao/accounts.rs:104-126`)是**纯 INSERT 无 ON CONFLICT**;对已存在的 account_id 二次插入触发 `UNIQUE constraint failed: accounts.id`,`create` 返回 `Err`。
6. `codex_oauth.rs:294-303` 捕获 `Err` 后 `cleanup_session` 并返回 HTTP 500 `db_save_failed`,新加密的 token 已构造但**未落库、被丢弃**。

**与报告的出入(仅锚点行号)**:报告 §3 P1-3 影响栏提到「oauth_codex 账号被通用 create 端点拒绝,accounts.rs:95-100」。该锚点实际位于 `http/api/accounts.rs:95-100`(API handler 层的 `create` 显式拒绝 `account_type == "oauth_codex"`),而非 `db/dao/accounts.rs`(DAO 层 89-102 是 `get` 函数)。此出入不影响 P1-3 的触发与修复,仅订正定位。

**既有可复用 helper**:`dao/accounts.rs` 已存在 `accounts::update(id, AccountUpdate)`(128-183),支持部分字段更新,含 `credentials_encrypted`/`name`/`extra_json`/`last_login_at` 等(嵌套 Option 区分「不更新」与「更新为 NULL」)。无需新增 DAO helper,直接复用。

**加密 AAD 一致性**:`crypto.encrypt` 以 `account_id_str` 作 AAD(`app-stack-conventions.md` AES-256-GCM 节)。重登录走 update 路径时 account_id 不变,AAD 一致,后续解密不受影响——upsert 在加密层是安全的。

## Requirements

- `handle_callback` 在写库前先用 `accounts::get(&app_state.db, &account_id_str)` 判定账号是否已存在。
- 已存在:走 `accounts::update`,覆盖 `credentials_encrypted`(新加密 BLOB)、`name`(email 或 "Codex 账号");不动 `account_type`/`platform`/`status`/`priority`(保持原值)。
- 不存在:维持现有 `accounts::create` 路径(首次登录行为不变)。
- update 路径成功后返回 HTTP 200(与 create 路径一致的 `{status, email, plan_type}` 响应),不返回 500。
- 不改变 `cleanup_session` 调用时机(成功/失败均清理)。
- 不引入回归:`accounts::create`/`accounts::update`/`accounts::get` 既有调用方(API handler、portability apply、oauth refresh)行为不变。

## Acceptance Criteria

- [ ] 新增测试:模拟同一 `chatgptAccountId` 二次登录(先 create 一行,再走 update 分支),断言返回 200 且 DB 中该 account_id 行的 `credentials_encrypted` 被更新为新 BLOB(非旧行)、`name` 更新、`account_type`/`platform` 保持 `oauth_codex`/`openai_codex`。
- [ ] 新增测试:首次登录(无既有行)仍走 create 路径,断言行被插入、返回 200。
- [ ] 现有 `codex_oauth.rs` / `dao/accounts.rs` / `http/api/accounts.rs` 相关测试全绿,无回归。
- [ ] `cargo test` + `cargo clippy -D warnings` + `cargo fmt --check`(本任务新增代码须 fmt 干净) + `tsc --noEmit` + `npm run build` 全绿。

## Out of Scope

- P2-22(OAuth 回调 state 不匹配时不清理 session,`codex_oauth.rs:207`)——另立任务。
- P2-23(初始登录不写 expires_at,`codex_oauth.rs:243`)——另立任务。
- P2-24(account_id 缺失时回退随机 UUID 导致重复账号,`codex_oauth.rs:265`)——另立任务。
- 其它 P1(P1-1/P1-2/P1-4/P1-5)——各独立任务。
- 既有 `cargo fmt` 漂移(报告 §7 已知限制,约 10 处)——不在本任务收敛,仅保证本任务新增/改动代码 fmt 干净。

## Notes

- 修复须对照 `app-stack-conventions.md`「Codex OAuth 约定」「AES-256-GCM 加密结构」「DAO 不做加密」三节落地:DAO 只做 SQL,加密在 service 层完成后传 BLOB;account_id 作 AAD;重登录 update 不改 account_type/platform。
- 安全考量:重登录必须用新 token 覆盖旧 `credentials_encrypted`,**不得**残留旧凭据或产生重复行(update 原地覆盖单行,天然满足)。
- 不采用「登录前删除同 account_id 旧记录后重建」方案:会丢失 `created_at`/`priority`/`last_login_at` 等列历史值,且多一次 DELETE+INSERT 不如 update 原子简洁。优先复用既有 `accounts::update` helper。
