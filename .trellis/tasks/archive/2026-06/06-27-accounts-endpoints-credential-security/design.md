# 账号端点与凭据安全设计

## 设计目标

在 app-shell 子任务的骨架基础上，新增账号、端点、凭据加密、Codex OAuth 登录与管理 API，不重写既有 HTTP/SQLite 分层。

## 总体架构

```text
agent-switch 进程
├── Tauri Runtime（已有）
├── Axum HTTP Server 127.0.0.1:42567（已有）
│   ├── /api/accounts          账号 CRUD + 登录触发
│   ├── /api/endpoints         端点 CRUD
│   ├── /api/auth/codex/login  启动 Codex OAuth 登录
│   └── /health, /claude-code/*, /codex/*, /v1/* （已有占位）
├── Codex OAuth 临时回调服务 127.0.0.1:1455
│   └── /auth/callback         接收授权码，交换 token
├── SQLite（已有）
│   ├── schema_migrations（已有）
│   ├── accounts               新增
│   └── endpoints              新增
└── 凭据保护层
    ├── 系统 Keychain 主密钥
    └── AES-GCM 加密/解密服务
```

## 技术栈与来源

| 领域 | 决策 | 来源 |
|------|------|------|
| 账号/端点数据模型 | 两层表：accounts + endpoints | `sub2api` accounts 表 + `ccs` 端点模型 |
| 凭据加密 | AES-GCM | `9router` 本地加密 + `ccs` Keychain（父任务确认） |
| 主密钥存储 | 系统 Keychain | `ccs`（父任务确认） |
| Codex OAuth | authorization_code + PKCE，端口 1455 | `9router` + `cpa` |
| Rust 加密库 | `aes-gcm` + `rand` | Rust 生态标准 |
| Rust Keychain 库 | `keyring` | Rust 生态跨平台 |
| Rust HTTP 客户端 | `reqwest` | Rust 生态标准，路由子任务复用 |
| OAuth 回调 | 临时 Axum 服务 on 1455 | `9router` / `cpa` |

## SQLite 迁移

在既有迁移框架（app-shell 子任务的 `db/migrations.rs`）基础上新增迁移 v2。

### 迁移 v2：accounts + endpoints

```sql
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  account_type TEXT NOT NULL,
  platform TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  credentials_encrypted BLOB,
  extra_json TEXT,
  priority INTEGER NOT NULL DEFAULT 0,
  last_login_at TEXT,
  last_error TEXT,
  last_error_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS endpoints (
  id TEXT PRIMARY KEY,
  account_id TEXT,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  protocol_type TEXT NOT NULL,
  api_key_encrypted BLOB,
  auth_mode TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  priority INTEGER NOT NULL DEFAULT 0,
  cooldown_until TEXT,
  last_success_at TEXT,
  last_failure_at TEXT,
  last_error_kind TEXT,
  extra_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_endpoints_account_id ON endpoints(account_id);
CREATE INDEX IF NOT EXISTS idx_endpoints_enabled_priority ON endpoints(enabled, priority);
```

## 凭据加密设计

### 主密钥

- 启动时从系统 Keychain 读取 service=`agent-switch`、account=`master-key` 的主密钥。
- 若不存在，生成 32 字节随机密钥并写入 Keychain。
- 若 Keychain 不可用（Linux 无 Secret Service 等），进入降级模式：
  - UI 明确提示"系统凭据管理器不可用"。
  - 不静默改明文存储。
  - 已加密凭据无法解密，要求用户重新录入。

### 加密流程

```text
明文凭据 JSON
  → 随机 12 字节 nonce
  → AES-256-GCM 加密 (key=主密钥, nonce, aad=account_id 或 endpoint_id)
  → nonce || ciphertext || tag 拼接存入 BLOB
```

### 解密流程

```text
BLOB
  → 拆分 nonce / ciphertext / tag
  → AES-256-GCM 解密 (key=主密钥, nonce, aad)
  → 明文凭据 JSON
```

### 凭据 JSON 结构

Codex OAuth 账号：

```json
{
  "access_token": "...",
  "refresh_token": "...",
  "id_token": "...",
  "expires_at": "2026-06-27T10:00:00Z",
  "account_id": "...",
  "email": "...",
  "plan_type": "..."
}
```

API Key 端点：

```json
{
  "api_key": "sk-..."
}
```

## Codex OAuth 流程

### 流程步骤

参考 `9router` / `cpa`：

1. 前端调用 `POST /api/auth/codex/login`。
2. 后端生成 PKCE code_verifier + code_challenge + state。
3. 后端启动临时 Axum 服务 on `127.0.0.1:1455`，路由 `GET /auth/callback`。
4. 后端返回授权 URL（含 code_challenge、state、redirect_uri=`http://localhost:1455/auth/callback`）。
5. 前端打开浏览器到授权 URL。
6. 用户在浏览器登录 OpenAI 账号并授权。
7. OpenAI 回调 `http://localhost:1455/auth/callback?code=...&state=...`。
8. 临时服务验证 state，用 code + code_verifier 交换 token。
9. 解析 id_token JWT 获取 account_id、email、plan_type。
10. 加密 token 凭据，写入 accounts 表。
11. 关闭临时服务，释放端口 1455。
12. 前端轮询或 SSE 获取登录结果。

### Codex OAuth 元数据（参考 9router）

- 授权端点：`https://auth.openai.com/oauth/authorize`
- Token 端点：`https://auth.openai.com/oauth/token`
- 回调端口：`1455`
- 回调路径：`/auth/callback`
- Redirect URI：`http://localhost:1455/auth/callback`
- Scope：参考 `9router` 的 Codex OAuth scope 配置。

### 临时回调服务

- 独立于主 Axum 服务（42567），用单独的 `tokio::net::TcpListener` 绑定 1455。
- 登录完成后立即 graceful shutdown 释放端口。
- 同一时刻只允许一个 Codex OAuth 登录会话。

## 管理 API 契约

### 账号

```text
GET    /api/accounts              列表
POST   /api/accounts               创建（API Key 类型）
GET    /api/accounts/{id}          详情（脱敏）
PUT    /api/accounts/{id}          更新
DELETE /api/accounts/{id}          删除
POST   /api/accounts/{id}/refresh  刷新 OAuth token（占位，本子任务可只刷新 Codex）
```

### 端点

```text
GET    /api/endpoints              列表
POST   /api/endpoints               创建
GET    /api/endpoints/{id}          详情（脱敏）
PUT    /api/endpoints/{id}          更新
DELETE /api/endpoints/{id}          删除
POST   /api/endpoints/{id}/toggle   启用/禁用
```

### Codex OAuth

```text
POST   /api/auth/codex/login       启动登录，返回授权 URL
GET    /api/auth/codex/status       查询当前登录会话状态
```

### 响应脱敏规则

- API Key 返回时只显示 `sk-****1234`（前缀 + 后 4 位）。
- OAuth token 不返回 token 值，只返回 `email`、`plan_type`、`expires_at`、`status`。
- `credentials_encrypted` / `api_key_encrypted` 字段永不返回到前端。

## Rust 模块边界

在 app-shell 既有分层上扩展：

```text
src-tauri/src/
├── db/
│   ├── migrations.rs           新增 v2 迁移
│   └── dao/
│       ├── accounts.rs         新增
│       └── endpoints.rs        新增
├── services/                   新增模块
│   ├── mod.rs
│   ├── crypto.rs               AES-GCM 加密/解密
│   ├── keychain.rs             主密钥读写
│   └── codex_oauth.rs          OAuth 流程
├── http/
│   ├── api/                    新增
│   │   ├── mod.rs
│   │   ├── accounts.rs
│   │   ├── endpoints.rs
│   │   └── auth.rs             Codex OAuth 登录触发
│   └── router.rs               更新：挂载 /api/accounts /api/endpoints /api/auth
└── app_state.rs                更新：持有加密服务句柄
```

设计原则：

- DAO 只做 SQL，不做加密。
- 加密集中在 `services/crypto.rs`。
- OAuth 流程在 `services/codex_oauth.rs`，不渗入 DAO 或 HTTP handler。
- HTTP handler 只做参数校验和响应脱敏，不直接拼 SQL 或加密。

## 前端设计

### 账号页面

- 列表：名称、平台、授权方式、优先级、状态、最后登录、操作。
- Codex OAuth 登录按钮：触发 `/api/auth/codex/login`，打开浏览器，轮询状态。
- 账号详情：显示 email、plan_type、expires_at、状态；不显示 token。
- API Key 账号：创建表单（名称、平台、API Key）。

### 端点页面

- 列表：名称、关联账号、base URL、协议、认证方式、优先级、启用状态、操作。
- 编辑表单：base URL、协议类型、认证方式、优先级、启用开关。
- API Key 输入：脱敏显示，编辑时显示占位。
- 启用/禁用开关。

### 前端模块

```text
src/
├── pages/
│   ├── AccountsPage.tsx        更新：真实列表 + 登录
│   └── EndpointsPage.tsx       更新：真实列表 + 编辑
├── components/
│   ├── accounts/
│   │   ├── AccountList.tsx
│   │   ├── CodexLoginButton.tsx
│   │   └── AccountForm.tsx
│   └── endpoints/
│       ├── EndpointList.tsx
│       └── EndpointForm.tsx
└── lib/
    └── api.ts                  扩展：accounts/endpoints/auth 客户端
```

## 安全边界

- 沿用父任务：本地服务不做 token/session 认证，仅绑定 127.0.0.1。
- Codex OAuth 临时回调服务也绑定 127.0.0.1:1455。
- 不记录请求正文、完整 headers、API Key、OAuth token。
- 凭据字段永不返回前端；UI 脱敏。
- OAuth state 校验防止 CSRF。
- PKCE 防止授权码拦截。

## 降级策略

- Keychain 不可用：
  - 启动时检测；UI 提示。
  - 已加密凭据无法解密；要求重新录入。
  - 不允许新增凭据写入（避免明文）。
- Codex OAuth 回调端口 1455 被占用：
  - 登录失败并提示用户释放端口。
  - 不自动换端口（与主服务 42567 同样原则）。
- OAuth token 交换失败：
  - 记录 last_error；UI 显示。
  - 不保存部分凭据。

## 后续子任务扩展点

- `model-management-refresh-alias`：端点模型刷新复用 endpoints 表和凭据。
- `tool-takeover-claude-code-codex`：Codex OAuth token 供 Codex 路由使用。
- `routing-failover-core`：使用 endpoints 的 cooldown/priority/last_error 字段。
- `openai-compatible-v1-endpoints`：使用 endpoints 的 protocol_type 和 auth_mode。
- `chain-testing-debugger`：使用 accounts/endpoints 数据构造真实链路测试。
- `import-export-settings`：加密导出包包含 accounts/endpoints 凭据；脱敏导出不包含。

## 重要取舍

- 选择 PKCE 流程而非 Device Code：与 `9router` / `cpa` 一致；Device Code 预留扩展点。
- 选择 `keyring` crate 而非各平台原生 API：跨平台一致，减少实现成本。
- 选择 AES-GCM 而非其他算法：Rust 生态成熟，认证加密防止篡改。
- 不单独建 account_groups 表：第一版用 priority 字段简化，避免过早引入分组复杂度。
