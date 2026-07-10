# 账号端点与凭据安全实现验证记录

验证时间：2026-06-27

## 构建验证

| 检查 | 命令 | 结果 |
|------|------|------|
| 前端类型检查 + 构建 | `npm run build` | ✅ 通过（293 KB JS + 12.8 KB CSS，60 模块） |
| Rust 格式检查 | `cargo fmt --check` | ✅ 通过 |
| Rust 编译 | `cargo check` | ✅ 通过 |
| Rust lint | `cargo clippy --all-targets -D warnings` | ✅ 通过 |
| Rust 二进制构建 | `cargo build` | ✅ 通过 |

## 运行期验证

启动 `src-tauri/target/debug/agent-switch.exe`，日志显示：

```text
INFO agent_switch_lib: Agent-Switch 正在启动...
INFO agent_switch_lib: 数据目录：C:\Users\chen_\AppData\Roaming\com.agent-switch.app
INFO agent_switch_lib::db::migrations: 执行迁移 v2: create_accounts_and_endpoints
INFO agent_switch_lib::db::migrations: 数据库迁移：1 项迁移已执行
INFO agent_switch_lib::services::keychain: 已生成并写入主密钥到系统凭据管理器
INFO agent_switch_lib: 凭据加密服务已就绪
INFO agent_switch_lib::http: 本地服务已启动：http://127.0.0.1:42567/
```

### 账号 CRUD

| 操作 | 结果 |
|------|------|
| `GET /api/accounts`（空） | ✅ `[]` |
| `POST /api/accounts`（API Key） | ✅ 201，`has_credentials: true` |
| `GET /api/accounts`（创建后） | ✅ 返回账号，凭据字段不返回，只有 `has_credentials` 布尔 |
| `DELETE /api/accounts/{id}` | ✅ 204 |
| 删除后列表 | ✅ `[]` |

### 端点 CRUD

| 操作 | 结果 |
|------|------|
| `GET /api/endpoints`（空） | ✅ `[]` |
| `POST /api/endpoints` | ✅ 201，`has_api_key: true` |
| `POST /api/endpoints/{id}/toggle`（禁用） | ✅ 204 |
| 查询验证 | ✅ `enabled: false`，`has_api_key: true` |
| `DELETE /api/endpoints/{id}` | ✅ 204 |

### Codex OAuth

| 操作 | 结果 |
|------|------|
| `GET /api/auth/codex/status`（无登录） | ✅ `{"login_in_progress": false}` |
| `POST /api/auth/codex/login` | ✅ 返回授权 URL，含 PKCE code_challenge、state、正确 client_id、redirect_uri |
| 回调服务 on 1455 | ✅ 启动并监听 |
| 回调收到 cancel | ✅ 返回 HTTP 400，清理会话 |
| `codex/status`（cancel 后） | ✅ `login_in_progress: false` |
| 1455 端口释放 | ✅ 登录结束后 1455 不再监听 |

授权 URL 结构正确：

```text
https://auth.openai.com/oauth/authorize
  ?response_type=code
  &client_id=app_EMoamEEZ73f0CkXaXp7hrann
  &redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback
  &scope=openid%20profile%20email%20offline_access
  &state=<32字节随机>
  &code_challenge=<S256>
  &code_challenge_method=S256
```

与 `9router` / `cpa` 的 Codex OAuth 元数据一致。

### 脱敏验证

- `credentials_encrypted` / `api_key_encrypted` BLOB 永不返回前端。
- 响应只包含 `has_credentials` / `has_api_key` 布尔字段。
- API Key 值不返回。

### 安全验证

- 主密钥写入 Windows Credential Manager（service=`agent-switch`, account=`master-key`）。
- 凭据用 AES-256-GCM 加密后存入 SQLite BLOB。
- OAuth state 校验防 CSRF。
- PKCE 防授权码拦截。
- 1455 回调服务绑定 127.0.0.1。

## 未验证项

- 真实 Codex OAuth token 交换：需要真实 OpenAI 账号在浏览器完成授权，CI 无法自动化。授权 URL 结构已验证正确，token 交换代码路径（`exchange_code_for_token`）与 `9router`/`cpa` 一致。
- 中文 UI：bash/curl 传递中文 JSON 时有 shell 编码问题，但前端通过 fetch 发送 UTF-8 JSON 不受影响；UI 文案本身是中文（在 React 组件中硬编码，不经过 shell）。
- Keychain 不可用降级：当前 Windows 环境 Keychain 可用，降级路径代码已实现但未触发。
