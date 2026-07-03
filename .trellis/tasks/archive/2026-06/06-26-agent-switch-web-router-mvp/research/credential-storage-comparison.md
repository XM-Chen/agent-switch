# 凭据存储与保护策略对比研究

## 研究问题

为 agent-switch 第一版设计 API Key、Codex OAuth token、账号 token 等敏感凭据的落盘策略，需要参考：

- `9router`：本地数据目录 / SQLite，并对 OAuth token、API key 等本地存储做加密。
- `ccs`：桌面应用形态，优先使用系统 Keychain / OS credential store，必要时回退本地凭据文件。
- `cpa`：CLI 工具形态，OAuth / 服务账号凭据通常序列化到本地 JSON 文件，并依赖目录权限保护。
- `sub2api`：服务端部署形态，敏感配置多来自 `.env` / 环境变量 / 配置文件 / 数据库加密字段，并强调文件权限。

## 四项目对比

| 项目 | 存储方式 | 保护方式 | 对 agent-switch 的启发 |
|------|----------|----------|------------------------|
| `9router` | 本地数据目录与 SQLite，包含 provider、alias、key、setting 等状态 | OAuth token 和 API key 加密后存储；数据目录便于备份 | agent-switch 应学习“本地数据库 + 加密敏感字段”，避免 SQLite 被复制后直接泄露凭据 |
| `ccs` | 桌面工具配置与凭据文件；macOS 上优先读取 Keychain，找不到再回退本地文件 | 使用系统 Keychain / OS credential store 保护 OAuth 凭据；回退文件主要依赖本地权限 | agent-switch 是 Tauri 桌面应用，最适合学习系统凭据管理器思路 |
| `cpa` | OAuth token、服务账号等凭据序列化到本地 JSON 文件 | 创建凭据目录时设置严格文件权限，例如仅所有者可访问 | 可学习“文件权限是最低保护层”，但第一版不应只依赖明文 JSON/SQLite |
| `sub2api` | `.env`、环境变量、配置文件、数据库字段 | 自动生成 secret、设置 `.env` 权限、部分字段加密和脱敏显示 | 可学习 secret 生成、脱敏显示和备份提示；但其服务端部署模型不完全适合本地桌面应用 |

## 用户决策

用户选择第一版采用：

> SQLite 加密字段 + 系统 Keychain/凭据管理器保存主密钥。

## 推荐给 agent-switch 第一版的落地方案

### 1. 存储边界

- SQLite 保存核心配置、账号、端点、路由、模型映射、请求摘要日志。
- API Key、OAuth access token、refresh token 等敏感字段在 SQLite 中必须加密后存储。
- 加密主密钥不直接保存在 SQLite 中，而是保存到系统凭据管理器。

### 2. 系统凭据管理器

第一版按平台使用：

- Windows：Credential Manager。
- macOS：Keychain。
- Linux：Secret Service / libsecret；如果不可用，进入明确降级状态。

### 3. 降级策略

系统凭据管理器不可用时，不应静默改为明文存储。第一版应采用明确降级：

- UI 显示“系统凭据管理器不可用，无法安全保存新凭据”。
- 已保存且可解密的凭据继续使用；不可解密则要求用户重新录入。
- 后续可提供用户显式确认的“不安全本地明文模式”，但第一版默认不启用。

### 4. 备份与迁移

- 数据库备份可以迁移配置、路由、模型映射和非敏感状态。
- 由于主密钥绑定本机系统凭据管理器，数据库复制到另一台机器后，敏感凭据可能无法解密。
- UI 和文档需要提示：迁移到新机器后可能需要重新录入 API Key 或重新登录 OAuth。

### 5. 脱敏与日志

- UI 中展示 API Key / token 时默认只显示脱敏值，例如前后少量字符。
- 日志、请求摘要、真实链路测试结果不得保存完整 API Key、OAuth token、Authorization header。
- 导出配置默认不包含敏感凭据；如后续支持带凭据导出，必须单独加密并明确提示风险。

## 结论

agent-switch 第一版采用综合方案：

> 学习 `ccs` 的系统 Keychain / OS credential store 思路，学习 `9router` 的本地加密存储思路，吸收 `cpa` 和 `sub2api` 对文件权限、脱敏和备份迁移的经验。第一版落地为“SQLite 加密敏感字段 + 系统凭据管理器保存主密钥 + 不静默明文降级 + 迁移后重新录入凭据提示”。
