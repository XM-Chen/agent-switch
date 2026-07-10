# Research: agent-switch 现状精确行为

- **Query**: 确认本仓库（agent-switch）切换时的字段级 merge 范围、是否有 per-provider 快照机制、proxy 模式下 settings.json 精确内容与外围文件读写
- **Scope**: internal（`E:/SynologyDrive/git_files/agent-switch`）
- **Date**: 2026-07-07

## 总览

agent-switch 的切换语义与 ccs **架构上根本不同**：

- ccs：provider 的 `settings_config` = 完整 `settings.json` 内容快照；切换 = 整文件覆盖 live + per-provider backfill + common config 全局层。
- agent-switch：provider 的 `settings_config` = 端点引用（`{endpoint_id, model, wire_api, ...}`）；切换 = 设 `is_current` + 用 `tool_takeover::enable`/`enable_direct` 把 `env.ANTHROPIC_*` 几个字段**字段级 merge** 进 live。

agent-switch **没有** per-provider backfill、**没有** common config snippet、**没有** live 的整文件覆盖。用户 hooks/permissions/statusLine 是**全局共享**的（因为 takeover 只动 `env.ANTHROPIC_*`，从不碰其他顶层键），不是 per-provider 的。

---

## 7. agent-switch 切换时字段级 merge 的精确范围

### 切换入口

`src-tauri/src/http/api/providers.rs:253-272`（`switch` HTTP handler）→ `perform_switch`（providers.rs:278-331）：

```rust
fn perform_switch<F>(db, id, takeover: F) -> Result<SwitchResponse, ...> {
    // 1. 查目标 provider
    let provider = providers::get(db, id)?...;
    // 2. 解析 app_type → Tool
    let tool = Tool::from_str(&provider.app_type).filter(|t| t.supports_takeover())?;
    // 3. 记录切换前 current（回滚用）
    let prev_current = providers::get_current(db, &provider.app_type)?.map(|p| p.id);
    // 4. 先设 is_current（DB partial unique index 互斥）
    providers::set_current(db, id)?;
    // 5. 按 mode 接管
    match takeover(&provider, tool) {
        Ok(warnings) => Ok(SwitchResponse { warnings }),
        Err(e) => { /* 6. 回滚 is_current */ }
    }
}
```

takeover 闭包（providers.rs:257-269）按 `provider.mode` 分流：
- `"direct"` → `tool_takeover::enable_direct`（写真实凭据）。
- 其它（`"proxy"` 及未知）→ `tool_takeover::enable`（写占位符）。

### `apply`（proxy 模式）保留/覆盖的键

`src-tauri/src/services/tool_takeover/claude_code.rs:56-85`：

```rust
pub fn apply(config_dir: &Path) -> Result<(), String> {
    let path = settings_path(config_dir);
    let agent_url = format!("{}{}", LOCAL_BASE, CLAUDE_CODE_SUFFIX);
    // 读原文件（可能不存在/解析失败 → 空 object）
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(c) => serde_json::from_str(&c).unwrap_or(Value::Object(serde_json::Map::new())),
        Err(_) => Value::Object(serde_json::Map::new()),
    };
    // 确保 env 是对象
    if !root.get("env").is_some_and(|v| v.is_object()) {
        root["env"] = Value::Object(serde_json::Map::new());
    }
    if let Some(env) = root.get_mut("env") {
        if let Some(obj) = env.as_object_mut() {
            obj.insert("ANTHROPIC_BASE_URL".to_string(), Value::String(agent_url));
            obj.insert("ANTHROPIC_AUTH_TOKEN".to_string(), Value::String(PLACEHOLDER_TOKEN.to_string()));
        }
    }
    let json_bytes = serde_json::to_vec_pretty(&root)?;
    atomic_write(&path, &json_bytes)?;
    Ok(())
}
```

**精确覆盖范围（proxy 模式）**：
- `env.ANTHROPIC_BASE_URL` ← `http://127.0.0.1:42567/claude-code`（`LOCAL_BASE + CLAUDE_CODE_SUFFIX`，`tool_takeover/mod.rs:17-19`）
- `env.ANTHROPIC_AUTH_TOKEN` ← `"agent-switch-managed"`（`PLACEHOLDER_TOKEN`，mod.rs:23）
- **保留**：所有其他顶层键（`hooks`/`permissions`/`statusLine`/`includeCoAuthoredBy`/...）和 `env` 内所有其他键（`ANTHROPIC_MODEL`/`CLAUDE_CODE_USE_BEDROCK`/...）。

**精确覆盖范围（direct 模式）**，`claude_code.rs:92-124`：
- `env.ANTHROPIC_BASE_URL` ← 真实 `cfg.base_url`（从 endpoint 解析）
- `env.ANTHROPIC_AUTH_TOKEN` ← 真实解密 `cfg.api_key`
- `env.ANTHROPIC_MODEL` ← `cfg.model`（若 `Some`）
- **保留**：同上，所有其他键不动。

### 关键差异：merge 是"读-改-写"而非"整文件覆盖"

agent-switch 的 `apply`/`apply_direct` 是**读 live → 改 env 几个键 → 写回**，保留所有其他键。这与 ccs 的 `write_live_snapshot`（整文件覆盖）截然相反。

**后果**：
- 用户在 `settings.json` 里加的 `hooks`/`permissions`/`statusLine` 永远不会被 agent-switch 切换触碰——**全局常驻，跨所有 provider 共享**。
- 无法实现 per-provider hooks（provider A 用一套 hooks，provider B 用另一套）——因为 `provider.settings_config` 不是 `settings.json` 全文，只是端点引用。
- agent-switch 的 `enable`/`enable_direct`/`disable`/`reapply` 全链都不读 `provider.settings_config` 的内容来写 live（direct 模式只读 `endpoint_id`/`model`/`wire_api`）。

### 有没有任何 per-provider 快照机制（哪怕雏形）？

**没有。** 已确认：
- `provider.settings_config` 是静态端点引用（`DirectSettings` 结构，`tool_takeover/mod.rs:129-138`：`endpoint_id` + `model` + `wire_api` + `requires_openai_auth`），不是 live 快照。
- 切换流程（`perform_switch` + `enable`/`enable_direct`）**不读取 live settings.json 回写到 provider 的 DB 记录**。
- agent-switch 里唯一的 "backfill" 是 `db::dao::providers::backfill_from_takeover`（providers.rs:335+），但它是**启动迁移**：当 takeover 已启用但无 current provider 时，造一个确定性 id `prov-backfill-<tool>` 的占位 provider 行（`settings_config = "{}"`）。**不抓取 live 内容**，与 ccs 的 backfill 语义无关（命名容易误导）。
- 切换时唯一对 live 的"备份"是 `backup_before_write`（`tool_takeover/mod.rs:480-541`）：把原 `settings.json` **文件复制**到 `data_dir/backups/tools/<tool>-settings.json-<ts>.bak`，仅在**首次接管写**（当前不是 AgentSwitch 态）时做。这是灾难恢复备份，不是 per-provider 快照，也不用于切回时还原。

---

## 8. agent-switch proxy 模式下 settings.json 的精确内容 + 外围文件读写

### proxy 模式 settings.json 精确内容

经 `tool_takeover::enable`（mod.rs:145-190）→ `claude_code::apply` 后，`~/.claude/settings.json` 形如：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567/claude-code",
    "ANTHROPIC_AUTH_TOKEN": "agent-switch-managed",
    ... (其他原有 env 键保留)
  },
  ... (其他原有顶层键保留，如 hooks/permissions/statusLine)
}
```

- `LOCAL_BASE = "http://127.0.0.1:42567"`（mod.rs:17）。
- `CLAUDE_CODE_SUFFIX = "/claude-code"`（mod.rs:19）。
- `PLACEHOLDER_TOKEN = "agent-switch-managed"`（mod.rs:23）。
- **不写真实凭据**（proxy 模式绝无真实 key；direct 模式才写解密后的真实 key，mod.rs:23 的注释明确："写入工具配置的鉴权占位符,绝不包含真实凭据"）。

### backup_before_write 的备份语义

`tool_takeover/mod.rs:480-541`：
- 检测当前 live 是否已是 `LiveCategory::AgentSwitch`（`claude_code::detect`，claude_code.rs:21-50）。若是 → **跳过备份**（R3.4，不产生新备份记录）。
- 否则复制 `~/.claude/settings.json` 到 `<data_dir>/backups/tools/claude-code-settings.json-<timestamp>.bak`。
- 在 `tool_takeover_backups` 表（`db/migrations.rs:115`）记一行：`original_path`/`backup_path`/`original_existed`/`takeover_target`/`created_at`。
- 原文件不存在时记 `original_existed=0`，不复制。
- **这是文件级全量备份，不是 DB JSON 备份**（与 ccs 的 `proxy_live_backup` 表不同）。

### disable / reapply 语义

`tool_takeover/mod.rs:311-362`（`disable`）：
- **direct 模式 disable**：不真正关闭，而是**回退到 proxy 接管**——重写 live 为占位符 + 本地代理 URL（`claude_code::apply`），清掉真实凭据，`mode='proxy'`、`active_provider_id=NULL`、`enabled` 保持 1。"不让用户裸奔"。
- **proxy 模式 disable**：`set_enabled(false)`，**不改写工具文件**（R7，mod.rs:359，测试 mod.rs:824-839 验证）。

`tool_takeover/mod.rs:368-415`（`reapply`）：mode-aware，direct 重写 direct（真实 key），proxy 重写 proxy（占位符）。direct 缺激活 provider 时报错，**绝不静默降级为 proxy**。

### 此模式下是否有任何对外围文件的读写

**没有。** 已确认（读 `tool_takeover/mod.rs` + `claude_code.rs` 全文 + grep `common_config`/`snippet`/`mcp`/`CLAUDE.md`/`skills`）：
- `tool_takeover` 模块**只**读写 `~/.claude/settings.json`（Claude）和 `~/.codex/config.toml` + `~/.codex/auth.json`（Codex）。
- **不碰** `~/.claude.json`（mcpServers 所在）、`~/.claude/CLAUDE.md`、`~/.claude/skills/`、`~/.claude/projects/`。
- agent-switch 代码库中**没有** common config snippet 机制（grep `common_config|config_snippet|snippet` 在 `src-tauri/src` 下无命中，除 ccs 导入器 `services/importers/ccs.rs` 读取 ccs DB 时解析 `settings_config` 字段）。
- **没有** `proxy_live_backup` 表（migrations.rs 无此表；takeover 用文件级 `.bak` 备份代替）。
- **没有** MCP sync service、Skill sync service、prompt file 管理——agent-switch 目前完全不管理外围文件。

### DB schema 要点

`src-tauri/src/db/migrations.rs`：
- `providers` 表（migration v7，migrations.rs:195-213）：`id, app_type, name, mode(='proxy'), settings_config, is_current, category, sort_index, notes, meta, created_at, updated_at`。**无 `common_config` 列，无 backfill/snapshot 列**。
- `tool_takeover` 表（migration v4/v8，migrations.rs:107-112 + 217-218）：`tool, enabled, mode(='proxy'), active_provider_id, last_applied_at, last_target, last_error`。
- `tool_takeover_backups` 表（migration v4，migrations.rs:115-127）：文件级备份记录。
- `settings`/`app_metadata` 表（migrations.rs:67-72）：无 common config 键。

---

## 关键文件清单

| 文件路径 | 作用 |
|---|---|
| `src-tauri/src/http/api/providers.rs:253-331` | `switch` + `perform_switch`：设 is_current + 按 mode 接管 + 失败回滚 |
| `src-tauri/src/services/tool_takeover/mod.rs:145-257` | `enable`/`enable_direct`：备份 + 写工具配置 + 持久化状态 |
| `src-tauri/src/services/tool_takeover/mod.rs:311-415` | `disable`/`reapply`：mode-aware 关闭/重应用 |
| `src-tauri/src/services/tool_takeover/mod.rs:480-541` | `backup_before_write`：文件级 `.bak` 备份 |
| `src-tauri/src/services/tool_takeover/claude_code.rs:56-85` | `apply`（proxy）：字段级 merge，只覆盖 `env.ANTHROPIC_BASE_URL`+`env.ANTHROPIC_AUTH_TOKEN` |
| `src-tauri/src/services/tool_takeover/claude_code.rs:92-124` | `apply_direct`（direct）：字段级 merge，覆盖 `env.ANTHROPIC_BASE_URL`+`ANTHROPIC_AUTH_TOKEN`+`ANTHROPIC_MODEL` |
| `src-tauri/src/services/tool_takeover/claude_code.rs:21-50` | `detect`：识别 live 当前指向（agent_switch/official/third_party/unconfigured/unrecognized） |
| `src-tauri/src/db/dao/providers.rs:335+` | `backfill_from_takeover`：启动迁移（造占位 provider），**非 ccs 式 live 快照** |
| `src-tauri/src/db/migrations.rs:195-213` | `providers` 表 schema |
| `src-tauri/src/db/migrations.rs:107-127,217-218` | `tool_takeover` + `tool_takeover_backups` 表 schema |

## Caveats / Not Found

- Codex 的 `apply`/`apply_direct`（`tool_takeover/codex.rs`）未逐行读，但结构对称（写 `config.toml` + `auth.json`）。
- agent-switch 是否有"切回时从 `.bak` 还原"的流程：`disable`（proxy 模式）**不还原**（只 set_enabled=false，R7）；`disable`（direct 模式）回退 proxy 而非还原原文件。即 `.bak` 备份目前**仅供排查/手动恢复**，无自动还原路径。这与 ccs `proxy_live_backup` 的自动还原（`restore_live_configs`）形成对比。
- agent-switch 的 `services/importers/ccs.rs` 是从 ccs DB 导入 provider 的工具，会把 ccs 的完整 `settings_config`（Claude settings.json 全文）转成 agent-switch 的 endpoint + direct provider 结构（`ccs.rs:423,545,613`：`extract_env` 抽出 base_url/api_key/model 建端点）。这是**一次性迁移**，不影响切换语义。
