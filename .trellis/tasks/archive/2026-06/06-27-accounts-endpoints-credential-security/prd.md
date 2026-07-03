# 账号端点与凭据安全

## 目标

建立 agent-switch 的账号/供应商管理、端点管理、凭据保护与 Codex OAuth 登录能力，为后续模型管理、路由、工具接管等子任务提供数据与认证基础。

## 父任务约束

父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`

本子任务必须遵守父任务已确认的跨模块约束：

- 上游供应商管理需要暴露两层模型：账号/供应商组层，以及端点层。
- 账号登录/账号管理部分优先学习 `9router`：面向账号或供应商组管理认证、登录状态、可用性与关联端点。
- 第一版 OAuth provider 仅实现 OpenAI Codex OAuth provider，参考 `9router` 的 Codex OAuth provider 元数据与登录流程。
- 主服务仍固定为 `http://127.0.0.1:42567`；Codex OAuth 登录期间允许临时启动参考 `9router` 的专用 callback 端口（例如固定端口 + `/auth/callback`），登录结束后释放。
- 第一版 OAuth 范围不扩展到 Anthropic、Google、OpenAI 通用 OAuth 或其他网页登录；其他账号类型先通过 API Key / Token 或后续版本处理。
- 端点管理部分优先学习 `ccs`：面向具体 base URL、API Key、协议格式、模型映射、优先级、启用状态和故障转移配置。
- 第一版存储策略学习 `ccs`：核心配置、供应商、端点、路由规则、自动接管状态、API Key、Codex OAuth token 和请求摘要日志存 SQLite。
- 第一版敏感凭据保护采用"SQLite 加密字段 + 系统 Keychain/凭据管理器保存主密钥"。
- 所有项目文档和 UI 默认使用中文。

## 已确认技术决策

### 数据模型

- 账号/端点两层模型，综合 `sub2api` 的 accounts 表结构与 `ccs` 的端点管理。
  - 参考来源：`sub2api` 的 accounts/account_groups 表 + `ccs` 的端点配置。
  - 取舍：两层模型符合父任务约束；account_groups 分组在第一版先用 priority 字段简化，不单独建分组表。
- `accounts` 表：id、name、account_type、platform、status、credentials_encrypted、extra_json、priority、last_login_at、last_error、last_error_at、created_at、updated_at。
- `endpoints` 表：id、account_id、name、base_url、protocol_type、api_key_encrypted、auth_mode、enabled、priority、cooldown_until、last_success_at、last_failure_at、last_error_kind、extra_json、created_at、updated_at。

### 凭据加密

- AES-GCM 加密 `credentials_encrypted` 和 `api_key_encrypted`。
  - 参考来源：综合 `9router` 的本地加密存储 + `ccs` 的系统 Keychain 思路（父任务已确认）。
- 主密钥保存到系统 Keychain（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
- 启动时从 Keychain 取主密钥；不可用时明确降级，不静默明文。
- 凭据字段在 SQLite 中只存密文。

### Codex OAuth 流程

- 采用 `9router` + `cpa` 的 authorization_code + PKCE 流程。
  - 参考来源：`9router` 的 Codex OAuth provider + `cpa` 的 PKCE/回调实现。
  - 取舍：第一版先实现 PKCE 流程（父任务已确认参考 9router）；预留 Device Code（`ccs` 风格）扩展点但不在本子任务实现。
- 临时启动回调端口 1455 + `/auth/callback`；登录结束后释放。
- 解析 JWT 获取 planType、accountID、email；加密存储 token。
- OAuth 范围仅 OpenAI Codex，不扩展到其他 OAuth provider。

### Rust 加密与 Keychain 库

- 加密：`aes-gcm` + `rand`。
  - 参考来源：Rust 生态标准对称加密库；agent-switch 独立工程选择。
- Keychain：`keyring` crate（跨平台 Windows Credential Manager / macOS Keychain / Linux Secret Service）。
  - 参考来源：Rust 生态跨平台凭据管理库；agent-switch 独立工程选择。
- 序列化：`serde_json`（已在前一子任务引入）。
- HTTP 客户端（OAuth token 交换）：`reqwest`（Rust 生态标准异步 HTTP 客户端）。
  - 参考来源：agent-switch 独立工程选择；后续路由子任务也会复用。
- URL 解析：`url` crate。

### 管理 API

- `/api/accounts`、`/api/accounts/{id}`：账号 CRUD + 登录触发。
- `/api/endpoints`、`/api/endpoints/{id}`：端点 CRUD + 测试连通性占位。
- `/api/auth/codex/login`：启动 Codex OAuth 登录。
- `/api/auth/codex/callback`：Codex OAuth 回调（在临时端口 1455，不在主端口 42567）。
- 所有管理 API 仍受父任务"第一版本地服务不做 token/session 认证"约束。

## 需求

- 实现 `accounts` 和 `endpoints` 两张表的 SQLite 迁移（在 app-shell 子任务的迁移框架基础上新增版本）。
- 实现 DAO 层：`accounts` 和 `endpoints` 的 CRUD。
- 实现凭据加密/解密服务：AES-GCM + Keychain 主密钥。
- 实现 Codex OAuth 登录服务：PKCE + 临时回调端口 1455 + token 交换 + JWT 解析 + 加密存储。
- 实现管理 API：账号 CRUD、端点 CRUD、Codex OAuth 登录触发。
- 实现前端页面：账号页面（列表 + 登录 + 状态）、端点页面（列表 + 编辑 + 启用/禁用）。
- UI 所有文案中文。
- 凭据在 UI 中脱敏显示（只显示前后少量字符）。
- 端点 `protocol_type` 预留 `anthropic` / `openai_chat` / `openai_responses` / `codex` / `custom`，但本子任务只做数据结构和 UI，不实现真实路由转发。
- `endpoints.cooldown_until` / `last_success_at` / `last_failure_at` / `last_error_kind` 字段预留，供后续路由子任务使用，本子任务只写入字段结构。

## 验收标准

- [ ] `accounts` 和 `endpoints` 表通过 SQLite 迁移创建。
- [ ] DAO 层可对 accounts/endpoints 进行完整 CRUD。
- [ ] 凭据加密服务可加密/解密；主密钥从系统 Keychain 读取；Keychain 不可用时明确降级。
- [ ] Codex OAuth 登录可启动临时端口 1455 + `/auth/callback`，完成 PKCE 流程，获取并加密存储 token。
- [ ] 登录结束后临时端口 1455 释放。
- [ ] 管理 API `/api/accounts`、`/api/endpoints`、`/api/auth/codex/login` 可用。
- [ ] 前端账号页面可查看账号列表、触发 Codex OAuth 登录、显示登录状态和脱敏凭据。
- [ ] 前端端点页面可查看端点列表、编辑端点、启用/禁用端点。
- [ ] 所有 UI 文案中文。
- [ ] 设计文档明确数据模型、加密方案、OAuth 流程、API 契约和降级策略。
- [ ] 实现计划列出迁移版本、关键文件、验证命令和回滚点。

## 暂不纳入本子任务

- Claude Code / Codex 的真实路由转发（routing-failover-core 子任务）。
- `/v1` 多端点真实转发（openai-compatible-v1-endpoints 子任务）。
- 模型刷新、alias、能力类型（model-management-refresh-alias 子任务）。
- 自动接管本地工具配置（tool-takeover-claude-code-codex 子任务）。
- 真实链路测试与调试器（chain-testing-debugger 子任务）。
- 导入/导出功能（import-export-settings 子任务）。
- Device Code 登录流程（预留扩展点，不在本子任务实现）。
- 非 Codex 的 OAuth provider（Anthropic / Google / OpenAI 通用 OAuth）。

## 开放问题

当前子任务不再保留阻塞性开放问题。技术栈和工程默认项已按用户要求，基于既有约束与参考项目自行确定；后续由用户集中 review 并指出不合理点。
