# 执行计划 — 修复重复 OAuth 登录同一 ChatGPT 账号 PK 冲突丢 token(P1-3)

## 前置

- [x] PRD / design 已定。
- [x] 代码事实已复核(`codex_oauth.rs:294` create 纯 INSERT、`dao/accounts.rs:128-183` 既有 update helper 可复用、AAD=account_id 一致)。

## 执行步骤

1. **配置上下文(1.3)** — `implement.jsonl` / `check.jsonl` 加真实 spec 条目(删掉 `_example` 占位行):
   - `implement.jsonl`:
     - `{"file": ".trellis/spec/guides/app-stack-conventions.md", "reason": "Codex OAuth 约定 / AES-256-GCM 加密结构(AAD=account_id) / DAO 不做加密 / Keychain 降级 503"}`
     - `{"file": ".trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md", "reason": "§3 P1-3 触发/影响/修复方向 + P2-22/23/24 边界"}`
   - `check.jsonl`:同 app-stack-conventions.md(契约校验基线)。
   - `task.py validate` 通过。

2. **激活任务(1.4)** — review gate 后 `task.py start 07-03-fix-codex-oauth-relogin-pk`。

3. **实现(2.1)** — 派 `trellis-implement` sub-agent(或 inline),改 `src-tauri/src/services/codex_oauth.rs`:
   - 抽 `upsert_codex_account(app_state, account_id_str, name, credentials_json) -> Result<(), String>` helper(design.md §可测 helper 抽取 的签名与实现)。
   - `handle_callback` 步骤 9-12(`269-315` 的 crypto 匹配 + NewAccount + create + Err 分支)替换为调用 `upsert_codex_account`;Err 分支仍走 `cleanup_session` + 500 `db_save_failed`(保持错误码不变,仅语义从「create 失败」拓宽为「upsert 失败」,message 链路不变)。
   - `crypto.as_ref() → None` 的 503 分支(`305-314`)由 helper 内 `ok_or("加密服务不可用")` 早期返回承接,message 透传;`handle_callback` 外层仍可保留 503 响应映射(若 helper 返回含「加密服务不可用」关键字的错误则映射 503 crypto_unavailable,否则 500 db_save_failed)。**实现时确认错误码映射不退化**:当前 305-314 是独立的 503 分支,helper 化后须保留该 503 语义。
   - 在 `codex_oauth.rs` 新增 `#[cfg(test)]` mod(若已存在则追加)4 个测试(design.md §测试用例):first_login_creates / relogin_updates_credentials / crypto_unavailable_returns_error / (可选)handler 集成。

4. **质量检查(2.2)** — 派 `trellis-check`:
   ```bash
   cd src-tauri && cargo test --quiet
   cd src-tauri && cargo clippy --quiet -- -D warnings
   cd src-tauri && cargo fmt --check    # 本任务新增/改动代码须 fmt 干净
   npx tsc --noEmit
   npm run build
   ```

5. **Spec 更新(3.3)** — 用 `trellis-update-spec` 在 `app-stack-conventions.md`「Codex OAuth 约定」节补充:重登录走 `accounts::update`(覆盖 credentials_encrypted/name)、首次走 `accounts::create` 的 upsert 契约;并订正报告 §3 P1-3 中「accounts.rs:95-100」实为 `http/api/accounts.rs:95-100`(DAO 层 89-102 是 get)的锚点出入。

6. **提交(3.4)** — `fix(oauth): upsert account on re-login instead of insert (P1-3)`。

## 验证命令

见步骤 4。基线:`cargo test` 当前 92 passed,修复后应 ≥ 94(新增 ≥ 2测试;辅助 helper 单测与可选 handler 集成按实际落地数)。

## 回滚点

- 单文件 `codex_oauth.rs` 改动 + DAO 零改动;`git revert` 单 commit 即可回滚。
- 若测试发现 update 路径破坏既有 oauth_refresh / portability apply 的 `accounts::update` 调用(不可能——本任务不改 DAO、不改其它调用方),回到 design 重评是否需改为 DB 级 `INSERT ON CONFLICT DO UPDATE`(但那需改 DAO,偏离「复用既有 helper」决策,优先保持本任务范围)。

## 风险文件

- `src-tauri/src/services/codex_oauth.rs:184-340`(`handle_callback`,重点 269-315 写库分支 + 抽 helper)。
- `src-tauri/src/db/dao/accounts.rs:89-183`(`get`/`create`/`update` 既有 helper,只复用不改)。
- `src-tauri/src/http/api/accounts.rs:95-100`(oauth_codex create 拒绝,仅锚点订正引用,不改代码)。
