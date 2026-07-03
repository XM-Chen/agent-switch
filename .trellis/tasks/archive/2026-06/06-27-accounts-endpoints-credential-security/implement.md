# 账号端点与凭据安全实现计划

## 前置条件

- 当前任务：`.trellis/tasks/06-27-accounts-endpoints-credential-security`
- 父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`
- 依赖已完成子任务：`.trellis/tasks/archive/2026-06/06-27-app-shell-local-service`（应用骨架、SQLite 迁移框架、Axum 服务、路径隔离、前端骨架）
- 不开始实现，直到用户 review 当前 `prd.md` / `design.md` / `implement.md` 并明确同意进入实现。

## 实现顺序

### 1. 新增 Rust 依赖

在 `src-tauri/Cargo.toml` 增加：

```toml
aes-gcm = "0.10"
rand = "0.8"
keyring = "3"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
url = "2"
base64 = "0.22"
sha2 = "0.10"          # PKCE code_challenge
uuid = { version = "1", features = ["v4"] }
```

验证：`cargo check` 通过。

### 2. 迁移 v2：accounts + endpoints 表

在 `src-tauri/src/db/migrations.rs` 的 `MIGRATIONS` 数组追加 v2 迁移，SQL 见 `design.md`。

关键文件：
- `src-tauri/src/db/migrations.rs`

验证：启动应用，日志显示"执行迁移 v2"，`accounts` / `endpoints` 表存在。

### 3. DAO 层

新增：
- `src-tauri/src/db/dao/accounts.rs`
- `src-tauri/src/db/dao/endpoints.rs`

在 `dao/mod.rs` 注册模块。

DAO 只做 SQL，不做加密。CRUD 字段包括加密 BLOB（原样存取）。

验证：单元测试或临时 Tauri command 验证 CRUD。

### 4. 凭据加密服务

新增：
- `src-tauri/src/services/mod.rs`
- `src-tauri/src/services/crypto.rs`：AES-256-GCM 加密/解密，nonce 随机，aad=记录 ID。
- `src-tauri/src/services/keychain.rs`：主密钥读写（service=`agent-switch`，account=`master-key`）。

更新 `app_state.rs`：持有加密服务句柄或主密钥。

降级策略：Keychain 不可用时返回明确错误，不静默明文。

验证：加密→解密往返一致；Keychain 读取成功。

### 5. Codex OAuth 服务

新增：
- `src-tauri/src/services/codex_oauth.rs`：
  - 生成 PKCE code_verifier / code_challenge / state。
  - 启动临时 Axum 服务 on `127.0.0.1:1455`，路由 `GET /auth/callback`。
  - 构建授权 URL，返回前端。
  - 回调接收 code，交换 token（`reqwest` POST 到 token 端点）。
  - 解析 id_token JWT（base64 解码 payload，不验签，仅取字段）。
  - 加密凭据，写入 accounts 表。
  - 关闭临时服务，释放 1455。

关键约束：
- 同一时刻只允许一个登录会话（`tokio::sync::Mutex`）。
- state 校验防 CSRF。
- 端口 1455 被占用时登录失败，不换端口。

验证：手动走一遍 OAuth 流程（需要真实 OpenAI 账号；CI 无法自动化）。

### 6. 管理 API

新增：
- `src-tauri/src/http/api/mod.rs`
- `src-tauri/src/http/api/accounts.rs`：CRUD + refresh。
- `src-tauri/src/http/api/endpoints.rs`：CRUD + toggle。
- `src-tauri/src/http/api/auth.rs`：`POST /api/auth/codex/login`、`GET /api/auth/codex/status`。

更新 `src-tauri/src/http/router.rs`：挂载 `/api/accounts`、`/api/endpoints`、`/api/auth`，替换现有 `/api/{*path}` 占位。

响应脱敏：API Key 显示 `sk-****1234`；OAuth token 不返回值，只返回元数据。

验证：`curl` 调用各端点。

### 7. 前端账号页面

更新：
- `src/pages/AccountsPage.tsx`：真实列表 + 登录入口。
- `src/components/accounts/AccountList.tsx`
- `src/components/accounts/CodexLoginButton.tsx`
- `src/components/accounts/AccountForm.tsx`

使用 TanStack Query 调用 `/api/accounts`。
Codex 登录：调用 `/api/auth/codex/login` 获取 URL → `window.open` → 轮询 `/api/auth/codex/status`。

### 8. 前端端点页面

更新：
- `src/pages/EndpointsPage.tsx`：真实列表 + 编辑。
- `src/components/endpoints/EndpointList.tsx`
- `src/components/endpoints/EndpointForm.tsx`

API Key 脱敏显示；启用/禁用开关。

### 9. 扩展 API 客户端

更新 `src/lib/api.ts`：
- `accounts` 客户端
- `endpoints` 客户端
- `auth` 客户端（codex login / status）

### 10. 验证

```bash
npm install
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

运行期验证：

```bash
# 启动应用
./src-tauri/target/debug/agent-switch.exe &

# 账号 API
curl -s http://127.0.0.1:42567/api/accounts
curl -s -X POST http://127.0.0.1:42567/api/accounts -H "Content-Type: application/json" -d '{"name":"测试","account_type":"apikey","platform":"custom"}'

# 端点 API
curl -s http://127.0.0.1:42567/api/endpoints
curl -s -X POST http://127.0.0.1:42567/api/endpoints -H "Content-Type: application/json" -d '{"name":"本地端点","base_url":"http://example.com","protocol_type":"openai_chat","auth_mode":"none"}'

# Codex OAuth 登录（手动）
curl -s -X POST http://127.0.0.1:42567/api/auth/codex/login
# 浏览器打开返回的 URL，完成登录
curl -s http://127.0.0.1:42567/api/auth/codex/status

# 确认 1455 已释放
curl -s -m 3 http://127.0.0.1:1455/auth/callback && echo "still open" || echo "released"
```

预期：
- `/api/accounts` 返回列表（空或含测试账号）。
- API Key 脱敏显示。
- OAuth 登录后 1455 端口释放。
- 数据库 `accounts` / `endpoints` 表有记录。

## 风险与回滚点

### 迁移失败

风险：v2 迁移 SQL 错误导致数据库半初始化。

处理：迁移在事务内执行；失败则启动失败；可删除 `agent-switch.db` 重置（第一版无生产数据）。

### Keychain 不可用

风险：Linux 无 Secret Service 或权限问题。

处理：启动检测；UI 明确提示；不静默明文；要求用户配置 Secret Service 或重新录入。

### Codex OAuth 回调端口 1455 被占用

风险：其他程序占用 1455。

处理：登录失败并提示；不自动换端口。

### OAuth token 交换失败

风险：网络问题或授权码过期。

处理：记录 `last_error`；UI 显示；不保存部分凭据。

### Codex OAuth 元数据准确性

风险：`9router` / `cpa` 的 Codex OAuth 端点/scope 可能变化。

处理：实现时以参考项目实际配置为准；若 OpenAI 更改 OAuth 流程，需更新 `codex_oauth.rs`。

## 完成标准

- `prd.md`、`design.md`、`implement.md` 已完成并经用户 review。
- 用户明确同意进入实现后，才能运行 `task.py start`。
- 实现完成后必须运行构建/检查/运行期验证，并报告真实结果。
- Codex OAuth 真实登录验证需要用户手动配合（真实 OpenAI 账号）。
