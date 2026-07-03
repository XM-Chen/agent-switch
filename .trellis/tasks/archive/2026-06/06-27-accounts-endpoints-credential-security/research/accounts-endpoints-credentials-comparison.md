# 账号端点与凭据安全对比研究

## 研究问题

为 agent-switch 第一版设计账号/供应商、端点、API Key/OAuth token 存储与 Codex OAuth 登录流程，需要综合参考：

- `9router`：账号/供应商组管理、Codex OAuth provider、本地加密存储。
- `ccs`：端点管理、Codex OAuth Device Code / 反向代理流程、OAuth Auth Center。
- `cpa`：Codex OAuth 回调端口 1455、PKCE、本地回调服务器、CodexTokenStorage。
- `sub2api`：accounts 表结构、credentials 加密字段、account_groups 分组与优先级。

## 四项目对比

### 账号/供应商管理

| 项目 | 账号模型 | 关键字段 | 层级关系 |
|------|----------|----------|----------|
| `ccs` | App-specific Provider / Universal Provider；OAuth Auth Center 统一管理 Codex/Copilot 账号 | API Key、Preset、Select Account、Quota | 供应商分两层；一个 OAuth 账号可被多个供应商引用 |
| `9router` | provider/account 模型；本地 db.json 存储 | accessToken、refreshToken、expiresAt | provider 下挂账号；账号可登录、刷新、设置可用性 |
| `cpa` | Auth 记录；CodexTokenStorage 实例 | IDToken、AccessToken、RefreshToken、AccountID、Email、planType | 认证记录与渠道关联 |
| `sub2api` | accounts 表 + account_groups 表 | id、name、platform、type(oauth/apikey)、credentials(加密)、extra、concurrency、priority、status | 账号可分配到多个分组，分组内有优先级 |

### 端点管理

| 项目 | 端点模型 | 关键字段 |
|------|----------|----------|
| `ccs` | 端点配置，支持 Full URL Endpoint Mode | base URL、API Key、协议格式、模型映射、优先级、启用状态、故障转移配置 |
| `9router` | provider 下的具体上游 | base URL、认证、模型前缀 |
| `cpa` | openai-compatibility / vertex-api-key 等渠道配置 | prefix、models、api-key、base-url |
| `sub2api` | channels 表 | base_url、protocol、model mapping、priority |

### API Key / OAuth Token 存储与保护

| 项目 | 存储方式 | 保护方式 |
|------|----------|----------|
| `ccs` | 本地数据目录；experimental_bearer_token 可选不覆盖官方 auth.json | Refresh Token 不上传、不支持导出；macOS 优先 Keychain |
| `9router` | 本地 db.json / SQLite | OAuth token 和 API key 加密后存储 |
| `cpa` | 本地 JSON 文件 | 目录 0700 权限；日志记录凭据保存操作 |
| `sub2api` | accounts.credentials 字段 | 加密存储；脱敏显示 |

### Codex OAuth Provider 登录流程

| 项目 | 流程 | 回调端口 | 关键细节 |
|------|------|----------|----------|
| `ccs` | Device Code 流程 | 无（Device Code 不需回调端口） | 8 位验证码 + `https://auth.openai.com/codex/device`；后台轮询；约 15 分钟有效期 |
| `9router` | authorization_code PKCE 流程 | 固定端口 1455 + `/auth/callback` | OAuthModal 构建 URL，启动 codex proxy 处理服务端会话 |
| `cpa` | authorization_code + PKCE | 端口 1455 + `/auth/callback` | RequestCodexToken 启动回调转发器，等待回调文件，交换 token，解析 JWT planType/accountID |
| `sub2api` | （偏服务端，Codex OAuth 不是其主线） | — | — |

## 关键发现

1. **Codex OAuth 有两种成熟流程**：
   - Device Code 流程（`ccs`）：不需回调端口，用户输入 8 位验证码，适合桌面应用。
   - authorization_code + PKCE 流程（`9router`、`cpa`）：需固定回调端口 1455 + `/auth/callback`，启动本地回调服务器。

2. **回调端口 1455**：`9router` 和 `cpa` 都使用 `http://localhost:1455/auth/callback` 作为 Codex OAuth 回调，这是 Codex OAuth 的约定端口。agent-switch 父任务已确认"允许临时启动参考 9router 的专用 callback 端口（例如��定端口 + `/auth/callback`），登录结束后释放"。

3. **凭据加密**：四个项目都强调凭据不能明文。agent-switch 父任务已确认采用"SQLite 加密字段 + 系统 Keychain/凭据管理器保存主密钥"。

4. **账号与端点分层**：`ccs` 的两层模型（供应商/端点）和 `sub2api` 的 account_groups 分组模型都值得借鉴。父任务已确认"上游供应商管理需要暴露两层模型：账号/供应商组层，以及端点层"。

## 推荐给 agent-switch 第一版的落地方案

### 1. 数据模型（SQLite 表结构）

参考 `sub2api` 的 accounts 表 + `ccs` 的端点模型 + agent-switch 父任务的两层约束。

```sql
-- 账号/供应商组层
CREATE TABLE accounts (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  account_type TEXT NOT NULL,          -- 'oauth_codex' | 'apikey'
  platform TEXT NOT NULL,              -- 'openai_codex' | 'anthropic' | 'openai_compat' | 'custom'
  status TEXT NOT NULL DEFAULT 'active', -- 'active' | 'disabled' | 'expired' | 'error'
  credentials_encrypted BLOB,          -- 加密后的凭据 JSON
  extra_json TEXT,                     -- 非敏感额外信息
  priority INTEGER NOT NULL DEFAULT 0,
  last_login_at TEXT,
  last_error TEXT,
  last_error_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- 端点层
CREATE TABLE endpoints (
  id TEXT PRIMARY KEY,
  account_id TEXT,                     -- 可关联到账号；纯 API Key 端点可无账号
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  protocol_type TEXT NOT NULL,         -- 'anthropic' | 'openai_chat' | 'openai_responses' | 'codex' | 'custom'
  api_key_encrypted BLOB,              -- 加密后的 API Key（纯 API Key 端点用）
  auth_mode TEXT NOT NULL,             -- 'apikey' | 'oauth_codex' | 'none'
  enabled INTEGER NOT NULL DEFAULT 1,
  priority INTEGER NOT NULL DEFAULT 0,
  cooldown_until TEXT,
  last_success_at TEXT,
  last_failure_at TEXT,
  last_error_kind TEXT,
  extra_json TEXT,                     -- 非敏感配置：超时、重试、自定义 header 等
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE SET NULL
);
```

### 2. 凭据加密

- `credentials_encrypted` 和 `api_key_encrypted` 使用 AES-GCM 加密。
- 主密钥保存到系统 Keychain（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
- 启动时从 Keychain 取主密钥；不可用时明确降级，不静默明文。
- 凭据字段在 SQLite 中只存密文，不存明文。

### 3. Codex OAuth 流程

采用 `9router` + `cpa` 的 authorization_code + PKCE 流程（父任务已确认参考 9router）：

- 临时启动回调端口 1455 + `/auth/callback`；
- 构建 Codex 授权 URL（PKCE）；
- 用户浏览器登录 OpenAI 账号；
- 回调接收授权码；
- 交换 access token / refresh token；
- 解析 JWT 获取 planType、accountID、email；
- 加密存储 token 到 accounts.credentials_encrypted；
- 登录结束后释放回调端口。

同时保留后续可选 Device Code 流程（参考 `ccs`）的扩展点，但第一版先实现 PKCE 流程。

### 4. 账号与端点分层

- 账号层（accounts）：管理认证身份，包括 Codex OAuth 账号、API Key 供应商组。
- 端点层（endpoints）：管理具体 base URL、协议、优先级、启用状态、冷却状态。
- 一个账号可关联多个端点；一个纯 API Key 端点可不关联账号。
- 端点的 `auth_mode` 决定使用账号 OAuth token 还是自身 API Key。

### 5. UI 关键字段（中文）

账号页面：
- 账号名称、平台、授权方式（OAuth / API Key）、优先级、状态、最后登录时间、最后错误。
- Codex OAuth：登录按钮、登录状态、账号邮箱、planType。

端点页面：
- 端点名称、关联账号、base URL、协议类型、认证方式、优先级、启用状态、冷却状态、最后成功/失败时间。

## 结论

agent-switch 第一版采用综合方案：

> 账号/端点两层模型综合 `sub2api` 的表结构与 `ccs` 的端点管理；凭据保护采用 SQLite 加密字段 + 系统 Keychain 主密钥（父任务已确认）；Codex OAuth 采用 `9router` / `cpa` 的 authorization_code + PKCE 流程，临时端口 1455 + `/auth/callback`，登录结束后释放；第一版先实现 PKCE 流程，预留 Device Code 扩展点。
