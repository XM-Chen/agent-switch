# 双模式接管改造（tool_takeover 扩展 + apply_direct）

## Goal

迁移 v8 给 `tool_takeover` 加 `mode` / `active_provider_id` 列；`services/tool_takeover` 支持 proxy/direct 双模式：proxy 走现有 `apply()`（指向本地代理 + 占位符），direct 走新 `apply_direct(provider)`（写真实 base_url + 解密凭据到工具配置，绕过代理）。`disable` 在 direct 态回退到 proxy 模式（不裸奔）。`detect` 增加 direct 识别。

## 关键设计决策（deliberate deviation from ccs）

**direct 模式凭据来源 = 引用 endpoint_id，不内联明文 key。**

- ccs 的 `settings_config` 内含明文 API key（ccs DB 不加密凭据）。
- agent-switch 现有 `endpoints.api_key_encrypted` 用 AES-256-GCM 加密（以 endpoint.id 为 AAD），这是 agent-switch 的身份特征（用户已选「保留 agent-switch 定位」）。
- 因此 direct 模式 provider 的 `settings_config` 结构为：
  ```json
  {
    "endpoint_id": "ep_xxx",
    "model": "claude-sonnet-4-6",
    "tool_overrides": { ... }
  }
  ```
  `apply_direct` 时读 endpoint → 用 crypto 解密 api_key → 写真实配置到工具文件。凭据永不以明文落 DB、永不重复存储、解密路径与现有 `auth_injector` 一致。
- 偏离理由：安全不退化；复用现有加密端点；providers 保持纯切换面。

## Requirements

### R1 迁移 v8：tool_takeover 加列
- 在 `MIGRATIONS` 末尾追加 v8（v7 已被子任务 1 占用）。
- ```sql
  ALTER TABLE tool_takeover ADD COLUMN mode TEXT NOT NULL DEFAULT 'proxy';
  ALTER TABLE tool_takeover ADD COLUMN active_provider_id TEXT;
  ```
  不加 FK 约束——SQLite ALTER TABLE ADD COLUMN 对 ON DELETE 支持有限；引用完整性在 app 层保证（删除 provider 时 app 层清空对应 active_provider_id）。与现有 `tool_takeover_backups` 无 FK 的处理一致。
- 迁移测试：v8 后 `PRAGMA table_info(tool_takeover)` 包含 mode/active_provider_id；现有 `fresh_db_runs_all_migrations_in_order` 通过。

### R2 DAO 扩展 `db/dao/tool_takeover.rs`
- `ToolTakeoverRow` 加 `mode: String`、`active_provider_id: Option<String>`。
- `get_state` SELECT 补两列。
- `upsert_state` 签名加 `mode: &str`、`active_provider_id: Option<&str>`（写入 INSERT + ON CONFLICT UPDATE）。
- 新增 `set_mode(db, tool, mode, active_provider_id)`：只改模式与激活 provider，不动 enabled。
- 现有 `set_enabled` 保留。
- 单测覆盖。

### R3 tool_takeover 服务层双模式
- `Tool` 枚举保持（ClaudeCode/Codex/OpenCode），`supports_takeover` 仍只 ClaudeCode+Codex。
- `enable(db, tool, mode, provider, crypto, data_dir)`：
  - mode=Proxy：走现有 `apply()`（写本地代理 URL + 占位符 token）。
  - mode=Direct：校验 provider.mode==Direct 且 settings_config 含 endpoint_id → 调 `apply_direct(config_dir, provider, crypto, db)`。
  - 写前备份（复用现有 `backup_before_write`，direct 态备份含真实凭据的文件）。
  - 持久化 `upsert_state`（enabled=1, mode, active_provider_id, last_target）。
- `disable(db, tool, data_dir)`：
  - 若当前 mode==Direct：**回退到 proxy 模式**——调 `apply()` 写本地代理配置（清除真实凭据），set mode='proxy', active_provider_id=NULL，enabled 保持 1。语义=「关闭直连，回到代理接管」，不裸奔。
  - 若当前 mode==Proxy：set enabled=0（现有 R7 行为，工具留在指向死代理，无真实凭据泄露）。
- `detect` 返回保持现有 5 态枚举（direct 态在 live 文件视角=ThirdParty，由 tool_takeover.mode 字段标识 direct，不新增枚举）。

### R4 apply_direct 实现
- `claude_code::apply_direct(config_dir, provider, crypto, db)`：
  - 从 settings_config 读 endpoint_id → 查 endpoint → 解密 api_key（AAD=endpoint.id）。
  - 写 `settings.json`：`env.ANTHROPIC_BASE_URL = endpoint.base_url`、`env.ANTHROPIC_AUTH_TOKEN = <解密 key>`、可选 `env.ANTHROPIC_MODEL = settings_config.model`。
  - 合并写入（保留其它键），原子写。
- `codex::apply_direct(config_dir, provider, crypto, db)`：
  - 同样读 endpoint → 解密。
  - 写 `config.toml`：`model_provider = <provider_id>`、`[model_providers.<id>]` 含 `base_url`/`wire_api`/`requires_openai_auth`。
  - 写 `auth.json`：`OPENAI_API_KEY = <解密 key>`（保留 tokens 等字段）。
- 凭据安全：解密后的 key 只短暂存在于内存，写入工具文件后不记日志（detect/日志不得回显真实 token）。

### R5 约束
- 不动 HTTP API（子任务 3）、不动代理管道（子任务 4）、不动前端（子任务 5）。
- proxy 模式行为与现状完全一致（零回归）。
- 遵循现有错误风格：`Result<T, String>`、中文消息。

## Acceptance Criteria

- [ ] 迁移 v8 追加在末尾，`fresh_db_runs_all_migrations_in_order` 与 `migration_versions_are_ascending` 通过，新增断言验证 mode/active_provider_id 列存在。
- [ ] `enable(proxy)` 产物与改造前一致（占位符 token + 本地代理 URL，无真实 key）——回归测试。
- [ ] `enable(direct)` 产物含真实 base_url + 解密 key，写入正确工具文件（claude settings.json / codex config.toml+auth.json）。
- [ ] `disable` 在 direct 态回退 proxy：工具文件被改写为本地代理 URL + 占位符，真实 key 被清除，mode='proxy'，enabled=1。
- [ ] `disable` 在 proxy 态：enabled=0，工具文件不动（R7）。
- [ ] proxy 模式下 settings.json/auth.json 绝不含真实 API key（仅占位符）——安全测试。
- [ ] apply_direct 在 endpoint 不存在或解密失败时报错且不部分写文件。
- [ ] `set_mode` DAO 只改 mode+active_provider_id，不动 enabled。
- [ ] 全量门禁：fmt 干净 / clippy -D warnings / cargo test 全绿。

## Notes

- direct 模式把真实凭据写入工具配置文件是**有意为之**（ccs 同款），但 agent-switch 的 DB 不存明文 key（偏离 ccs，见上方决策）。
- 现有 `auth_injector.rs` 的解密逻辑（AAD=endpoint.id）可参考/复用。
- 备份机制不变：direct 写前也备份原文件。
