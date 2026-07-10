# 导入导出与设置

## 目标

为 agent-switch 提供**配置导入导出**与**设置页扩展**能力：支持两种导出模式（本机加密完整备份 / 可迁移脱敏配置导出），强制加密、明确风险提示、定义导入冲突策略，并在设置页提供导入导出入口与应用级设置。完成后用户可在本机安全备份完整配置（含凭据），或把脱敏配置迁移到另一台机器。

## 背景与边界

- 父任务：`06-26-agent-switch-web-router-mvp`,子任务拆分第 8 项「导入导出与设置」。
- 这是 MVP 收尾子任务,前 7 个子任务已落地:app-shell、accounts/endpoints/凭据加密、模型管理、工具接管、路由故障转移、v1 多端点、链路测试调试器。
- 范围:本机加密完整备份、可迁移脱敏配置导出、导入冲突处理、设置页扩展。

### 已就位基础（代码确认）

- `CryptoService::encrypt(plaintext, aad)` / `decrypt(blob, aad)`:AES-256-GCM,`nonce(12) || ciphertext || tag` 结构(`services/crypto.rs`)。
- 主密钥在系统 Keychain:`keychain::ensure_master_key()` / `load_master_key()`(`services/keychain.rs`)。
- `app_metadata` key-value 表可存设置项(`db/dao/app_metadata.rs`)。
- 现有 `SettingsPage` 已有「模型自动刷新」开关,导入导出 UI 应集成于此(`pages/SettingsPage.tsx`)。
- 可导出的数据表:`accounts`、`endpoints`、`endpoint_models`、`model_aliases`、`route_settings`、`tool_takeover`(状态)、`request_logs`(日志,默认不导出)。
- 凭据加密结构:endpoint `api_key_encrypted` = `{"api_key":"..."}`;account `credentials_encrypted` = `CodexCredentials`(含 access/refresh token)。

### 父任务已确认的导入导出规则（直接继承）

- 完整加密导入/导出包**允许**含 API Key / OAuth access/refresh token;**必须强制加密**,不得提供未加密导出敏感凭据的路径。
- 两种导出模式:
  - **本机加密完整备份**:系统 Keychain 主密钥加密,允许含凭据,适合本机/同凭据环境恢复,不保证跨机器恢复敏感凭据。
  - **可迁移脱敏配置导出**:不含 API Key/OAuth token/Authorization header/系统主密钥/请求日志/媒体内容/自动接管备份文件;含账号/端点非敏感元数据、base URL、协议类型、启用状态、优先级、custom 模型、alias、路由规则、可迁移 UI 配置。导入后凭据进入缺失状态。
- 导入后自动接管开关**不得**自动恢复为开启,**不得**自动写入 Claude Code/Codex 配置;统一关闭或显示「曾开启,需重新确认」。
- 导出包需定义:版本、加密算法、密码/密钥策略、弱密码提示、导入冲突处理、导入后是否立即启用端点和自动接管。

## 技术决策（最优方案,参考四项目）

> **原则**:取四个参考项目(ccs/sub2api/9router/cpa)的最优解,不取最简。

### D1 导入冲突策略 = 双模式分治(ccs 恢复 + sub2api 结构化 merge)

调研结论:**只有 ccs 和 sub2api 在配置导入导出上有实质参考**。
- ccs:SQL 导出/导入 + 数据库二进制快照备份,偏**整库覆盖式恢复**(`database/backup.rs`、`commands/import_export.rs`);导出时跳过日志类表(`SYNC_SKIP_TABLES`:proxy_request_logs/stream_check_logs/...)。
- sub2api:账号/session 结构化导入,**merge/normalize/preserve** 思路(`importData`、`importCodexSessions`)——解析→规范化→合并已有→清除过期 refresh 字段。
- 9router/cpa:无完整配置导入导出主线,不作冲突参考。

最优落地 = **按导出模式分治**:

| 数据类型 | 完整备份导入(replace 优先) | 脱敏导入(merge 优先) |
|---|---|---|
| 账号元数据 | replace | merge(按 name+account_type+platform) |
| 端点元数据 | replace | merge(按 name+base_url+protocol_type) |
| API Key/OAuth token | 主密钥可解密则恢复 | **skip(脱敏包本就不含)** |
| custom models | replace | merge/upsert |
| aliases | replace | merge/upsert |
| route_settings | replace | merge/upsert |
| tool_takeover 状态 | **导入后强制关闭** | **导入后强制关闭** |
| request_logs | skip | skip |
| 测试/调试数据 | skip | skip |
- 完整备份导入前**自动创建一次当前库本地备份**(ccs 恢复前备份思路)。
- merge 的匹配键:命中则更新非敏感字段,未命中则新增;导出内原始 ID 作为辅助匹配。

### D2 导出包加密 = 双密钥策略(ccs 主密钥 + 密码管理器式 Argon2id)

两种模式加密目标不同,故用不同密钥派生:

- **完整备份**:系统 Keychain 主密钥直接作为 AES-256-GCM key。绑定本机/同凭据环境(父 PRD 明确)。主密钥丢失则无法解密。
- **脱敏迁移**:用户设置**导出密码** → **Argon2id** KDF 派生 key → AES-256-GCM。可跨机器解密(脱敏包不含敏感凭据,但父 PRD 要求强制加密)。弱密码(< 8 字符或低熵)给出 UI 警告但不强制阻止。

> 取舍:父 PRD 说完整备份「不要求用户设置导出密码」(用主密钥),脱敏迁移若也用主密钥则无法跨机器解密——矛盾。最优解是脱敏迁移改用密码派生密钥,既满足「强制加密」又实现「可迁移」。这正是父 PRD 自述的「偏密码管理器方向的高级方案」。

### D3 导出包容器格式 = 版本化 JSON + gzip 压缩 + AES-256-GCM

```text
ExportPackage {
  format_version: u32,         // 当前 = 1
  mode: "full_backup" | "portable",
  algo: "AES-256-GCM",
  kdf: "none"(主密钥) | "argon2id"(密码),
  kdf_params: { salt, m_cost, t_cost, p_cost }(仅 argon2id),
  nonce: base64,
  created_at, app_version,
  ciphertext: base64,          // gzip(JSON payload) 加密后
}
```
- payload 明文 = 各表 JSON 数组(脱敏模式剔除凭据列)。
- 压缩(flate2 gzip)在加密前,减小体积(端点多/模型多时)。
- 文件扩展名 `.asbak`(完整备份) / `.ascfg`(脱敏),实质都是上述 JSON 文本。

### D4 导出导入入口 = 集成 SettingsPage + 后端 API

- 设置页新增「配置导入导出」卡片:两个导出按钮(完整备份/脱敏)、一个导入按钮、风险提示文案。
- 导出走浏览器下载(后端返回包文本,前端触发保存);导入走文件选择 + 文本上传。
- 后端 API:`POST /api/settings/export`(body {mode, password?})、`POST /api/settings/import`(body {package, password?, conflict_mode})、`GET /api/settings/export/preview`(可选,导入前预览包内容统计)。

## 需求

### R1 完整加密备份导出(AC1, AC4, AC9)
- R1.1 导出含全部配置表 + 敏感凭据(解密后重新装包,或直接装入加密 BLOB——见 design)。
- R1.2 用系统主密钥 AES-256-GCM 加密;主密钥不可用时返回 503 并提示。
- R1.3 包内标注 mode=full_backup、format_version、created_at、app_version。

### R2 可迁移脱敏导出(AC2, AC10)
- R2.1 导出剔除:API Key、OAuth token、Authorization、主密钥、请求日志、媒体、接管备份文件。
- R2.2 含:账号/端点非敏感元数据、base_url、protocol_type、enabled、priority、custom models、aliases、route_settings、可迁移 UI 设置（仅偏好类 `auto_model_refresh_enabled`；排除 `last_model_sync_at`/`last_model_sync_error` 等本机运行状态快照）。
- R2.3 用户设置导出密码 → Argon2id → AES-256-GCM;弱密码给警告。

### R3 完整备份导入(AC3, AC5, AC6, AC7)
- R3.1 主密钥解密;失败(密钥不匹配/包损坏)给可读错误,不崩溃。
- R3.2 replace 优先恢复各表;凭据可解密则恢复。
- R3.3 导入前自动创建当前库本地备份。
- R3.4 自动接管状态强制关闭,不写工具配置。

### R4 脱敏导入(AC8, AC6, AC7)
- R4.1 导出密码 → Argon2id 解密;密码错误给可读错误。
- R4.2 merge 优先:按匹配键 upsert 非敏感字段;凭据进入缺失状态。
- R4.3 自动接管状态强制关闭。

### R5 导入冲突与安全(AC6, AC7, AC11)
- R5.1 按 D1 矩阵处理冲突。
- R5.2 request_logs、测试数据永不导出/导入。
- R5.3 导入为事务:任一步失败回滚,不留半成品。

### R6 设置页 UI(AC12, 中文)
- R6.1 「配置导入导出」卡片:完整备份导出、脱敏导出(带密码输入)、导入(带密码输入+冲突模式)。
- R6.2 明确风险提示:完整包含凭据绑定本机、脱敏包跨机器需重录凭据、导入会覆盖/合并现有配置、自动接管导入后关闭。
- R6.3 弱密码警告。
- R6.4 所有文案中文。

## 验收标准

- [ ] AC1:完整备份导出生成主密钥加密包,含全部表 + 凭据,可被本机重新导入还原。
- [ ] AC2:脱敏导出包不含任何 API Key/OAuth token/Authorization/主密钥/日志/媒体;含非敏感元数据与 alias/route/custom model。
- [ ] AC3:完整备份导入用主密钥解密,恢复账号/端点/凭据/模型/alias/route_settings/端点启用状态。
- [ ] AC4:完整备份导出在主密钥不可用时返回 503,UI 可读提示,不崩溃。
- [ ] AC5:完整备份导入前自动创建当前库本地备份。
- [ ] AC6:任一导入完成后自动接管状态统一关闭,不写 Claude Code/Codex 配置。
- [ ] AC7:导入为事务,失败回滚,不留半成品。
- [ ] AC8:脱敏导入用导出密码 Argon2id 解密;merge 不覆盖本机已有敏感凭据。
- [ ] AC9:完整备份包标注 format_version/mode/created_at/app_version。
- [ ] AC10:脱敏导出弱密码触发 UI 警告(不强制阻止)。
- [ ] AC11:request_logs 与测试数据在任何模式下都不被导出或导入。
- [ ] AC12:设置页有导入导出卡片,风险提示完整,文案中文。
- [ ] AC13:质量门——`cargo fmt --check`、`cargo check`(0 error)、`cargo clippy`、`npm run build`。

## 暂不纳入范围

- 云同步 / WebDAV(父 PRD 第一版不做云同步)。
- 订阅源导入与批量转换(父 PRD 暂不纳入)。
- 一键从备份恢复工具配置(接管语义已定:导入不写工具配置)。
- 增量/差异导出、导出包签名验证(后续可加)。
