# Research: ccs 切换语义（回填保护 + Common Config + 代理接管）

- **Query**: 精确研究 ccs 的回填保护、Common Config Snippet、代理接管模式三层实现
- **Scope**: internal（ccs 仓库 `E:/SynologyDrive/git_files/cc-switch`）
- **Date**: 2026-07-07

## 总览

ccs 的 Claude Code 切换语义由三个独立机制叠加：

1. **per-provider 回填（backfill）**：切走前把整个 live `settings.json` 抓回当前 provider 的 DB `settings_config` 列。切回时整体还原。
2. **Common Config Snippet**：per-app-type 的全局 JSON 片段，每次写 live 时 deep-merge 到 provider 配置之上。
3. **代理接管（takeover）**：用 `PROXY_MANAGED` 占位符 + 本地代理 URL 替换 `env` 里的凭据/URL，外围配置（hooks/permissions/statusLine）保留。

三者关系：common config 是 always-merge 全局层；backfill 是 per-provider 层；takeover 是 proxy 模式下对 `env` 的字段级改写层。takeover 与前两者**正交**——进入 takeover 时 common config 仍生效；takeover 模式下的 provider 切换走 `hot_switch_provider`，不做 per-provider backfill，而是更新 live_backup + 重写 takeover env 字段。

---

## 决策 A 所需事实

### 1. 回填保护（backfill）精确机制

**切走前抓取什么**：整个 live `settings.json`（JSON 对象全文），通过 `read_live_settings(app_type)` 读取。
- 文件：`src-tauri/src/services/provider/live.rs:958-986`（Claude 分支：`get_claude_settings_path()` + `read_json_file`）。
- 不是特定字段集合，是整文件。

**存在哪**：写回**当前 provider**（切走前的那个）的 `settings_config` 列，在 `providers` 表内（不是独立快照表/文件）。
- 文件：`src-tauri/src/services/provider/mod.rs:1684-1702`：
  ```rust
  if let Ok(live_config) = read_live_settings(app_type.clone()) {
      if let Some(mut current_provider) = providers.get(&current_id).cloned() {
          current_provider.settings_config =
              strip_common_config_from_live_settings(
                  state.db.as_ref(), &app_type, &current_provider, live_config,
              );
          if let Err(e) = state.db.save_provider(app_type.as_str(), &current_provider) {
              // ...backfill_failed:{current_id}
          }
      }
  }
  ```
- DB schema：`src-tauri/src/database/schema.rs:27-43`，`providers` 表有 `settings_config TEXT NOT NULL`、`meta TEXT`，**没有专门的 backfill/snapshot 列**。

**切回时怎么恢复**：**整文件还原**（不是字段级）。切回 provider A 时，`write_live_with_common_config` → `write_live_snapshot`，把 A 的 `settings_config`（含此前 backfill 的快照）+ common config（deep-merge）整体 `write_json_file` 覆盖 `settings.json`。
- `src-tauri/src/services/provider/live.rs:509-529`（`write_live_with_common_config`）、`713-719`（`write_live_snapshot` 的 Claude 分支）：
  ```rust
  AppType::Claude => {
      let path = get_claude_settings_path();
      let settings = sanitize_claude_settings_for_live(&provider.settings_config);
      write_json_file(&path, &settings)?;
  }
  ```
- `sanitize_claude_settings_for_live`（live.rs:24-34）仅剥离 4 个内部字段（`api_format`/`apiFormat`/`openrouter_compat_mode`/`openrouterCompatMode`），其余原样落盘。

**快照属于谁**：属于**切走前的当前 provider**（`current_id`）。`mod.rs:1676` `let current_id = crate::settings::get_effective_current_provider(...)`；`mod.rs:1679` 仅当 `current_id != id`（切到别的 provider）才 backfill。即将切入的 provider 的 `settings_config` 不被改写。

**backfill 的预处理**：`strip_common_config_from_live_settings`（live.rs:531-571）会先 deep-remove common config 片段，再存 DB，避免 common config 在 DB 里重复累积。对 Codex 还会 `restore_codex_settings_for_backfill`（live.rs:573-600）处理 bearer token；Claude 是直通。

---

### 2. Common Config Snippet 精确机制

**存在哪**：DB `settings` 表（key-value），键名 `common_config_<app_type>`（如 `common_config_claude`）。
- `src-tauri/src/database/dao/settings.rs:62-64`：`get_config_snippet(app_type)` = `get_setting(&format!("common_config_{app_type}"))`。
- `settings.rs:121-136`：`set_config_snippet(app_type, snippet)` —— `Some(value)` 写入，`None` 删除键。
- 迁移 seeding：`src-tauri/src/database/migration.rs:217-241`，从旧 `common_config_snippets.{claude,codex,gemini}` 迁移到 settings 表。

**默认值**：`None`（未设置）。有自动抽取机制 `should_auto_extract_config_snippet`（settings.rs:92-95）：当 snippet 为 None 且未被用户显式清空（`is_config_snippet_cleared`）时，允许从 live 自动抽取。用户显式清空通过 `set_config_snippet_cleared` 标记 `common_config_<app>_cleared = "true"`。

**deep-merge 时机和优先级**：
- 入口 `build_effective_settings_with_common_config`（live.rs:483-507）：以 `provider.settings_config` 为 base，common config 片段 deep-merge **覆盖在上**（source 赢冲突）。
  ```rust
  let mut effective_settings = provider.settings_config.clone();
  if provider_uses_common_config(app_type, provider, snippet.as_deref()) {
      if let Some(snippet_text) = snippet.as_deref() {
          match apply_common_config_to_settings(app_type, &effective_settings, snippet_text) {
              Ok(settings) => effective_settings = settings, ...
```
- `apply_common_config_to_settings`（Claude 分支，live.rs:436-443）：`json_deep_merge(&mut result, &source)`，source（snippet）覆盖 target（provider settings）。
- 然后合并后的 effective settings 整体覆盖写 `settings.json`。
- **优先级**：common config > provider settings_config。但两者都**整体覆盖** live `settings.json`（live 上原有的、不在 provider settings 也不在 common config 的字段会丢失）。

**per-provider 开关**：`provider.meta.common_config_enabled`（三态）。
- `provider_uses_common_config`（live.rs:354-369）：
  - `Some(true)` → 启用（且 snippet 非空）。
  - `Some(false)` → 不启用（显式 false 优先于 legacy 检测）。
  - `None` → legacy 子集检测：若 provider 的 `settings_config` 已经包含 snippet 作为子集（`settings_contain_common_config`，live.rs:309-352），视为启用。
- 保存 provider 时 `normalize_provider_common_config_for_storage`（live.rs:602-637）会在 `common_config_enabled == true` 时把 snippet 从 `settings_config` 中 deep-remove，避免重复存储。

**和回填保护的关系**：
- common snippet = always-merge 的全局层（per-app-type）。
- backfill = per-provider 层（存于 `providers.settings_config`）。
- backfill 时 **剥离** common config（避免重复），写 live 时 **重新 merge** common config。
- 因此 common config 字段在每个 provider 的 DB 快照里都不存在，但每次写 live 都从 settings 表重新合并上去——全局一致。

---

### 3. 切换是整文件覆盖还是字段级 merge？`write_live_snapshot` 写什么？排序键的意义？

**整文件覆盖**，不是字段级 merge。
- `write_live_snapshot`（Claude 分支，live.rs:715-719）：`write_json_file(&path, &settings)`，整文件覆盖。
- `write_live_with_common_config`（live.rs:509-529）：先构造 `effective_provider`（settings_config + common config deep-merge），再调 `write_live_snapshot`。
- `write_json_file` 见 `src-tauri/src/config.rs`（标准实现，原子写）。

**排序键**：`providers.sort_index`（schema.rs:35）是** provider 列表展示顺序**，与 `settings.json` 内字段顺序无关。`settings.json` 的 JSON 键顺序由 serde_json `Map` 的插入顺序保留（BTreeMap 在 ccs 的 `json!` 宏下实际是保留插入序的 `serde_json::Map`，除非启用 `preserve_order` feature 否则按字母序——需注意，但对语义无影响，Claude Code 读 JSON 不依赖键顺序）。**排序键对切换语义无直接影响**。

---

### 4. 用户自加的 hooks/permissions/statusLine 在切换时的命运

由于切换是**整文件覆盖**：
- 用户在 provider A 激活时往 `settings.json` 加了 `hooks`/`permissions`/`statusLine`。
- 切 A→B 时：backfill 把 live（含这些字段）存入 A 的 `settings_config`；然后写 B 的 `settings_config` + common config 到 live。**这些字段从 live 消失**（除非 B 的 `settings_config` 也有，或在 common config 里）。
- 切回 B→A：backfill 把 live（B 的状态）存入 B；写 A 的 `settings_config`（含原 hooks）+ common config 到 live。**hooks 回来了**。
- 结论：用户自加字段**按 provider 维度保留**（存于各 provider 的 `settings_config`），切换时从 live 暂时消失，切回原 provider 时恢复。
- **例外**：若字段在 common config snippet 里，每次写 live 都重新 merge，**全局常驻**（不随 provider 切换消失）。
- **丢失条件**：字段既不在"切走前 provider 快照"（被 backfill 捕获）也不在 common config → 不会丢失（只要 backfill 成功，它进了当前 provider 的快照）。**真正丢失的场景**：backfill 失败（`mod.rs:1699` `backfill_failed:{current_id}` warning，但切换继续），或 live 文件不可读。

---

## 决策 B 所需事实

### 5. 代理接管模式（PROXY_MANAGED 占位符 + 本地代理 URL）

**进入 takeover**（`start_with_takeover`，`src-tauri/src/services/proxy.rs:439-497`）：
1. `backup_live_configs`（proxy.rs:1060-1093）：把每个 app 的 live `settings.json` 整体序列化为 JSON 存入 DB `proxy_live_backup` 表（`schema.rs:249-255`：`app_type TEXT PRIMARY KEY, original_config TEXT, backed_up_at TEXT`）。
2. `sync_live_to_providers`：把 live 里的真实 token 同步到 DB（类似 backfill）。
3. `set_live_takeover_active(true)`：在 `proxy_config` 表（`schema.rs:124-137`，`live_takeover_active` 列）置接管标志，崩溃恢复用。
4. `takeover_live_configs`（proxy.rs:1150-1214）：**改写 live**。
5. `start()`：启动代理服务器。

**Claude takeover 写什么**（`apply_claude_takeover_fields_for_provider`，proxy.rs:140-193；调用点 proxy.rs:1154-1163）：
- 先 `claude_provider_with_effective_settings`（proxy.rs:1156，调 `build_effective_settings_with_common_config`）——**common config 仍然生效**。
- 然后**字段级改写 live**（不是整文件覆盖）：
  - `env.ANTHROPIC_BASE_URL = proxy_url`（本地代理 URL，如 `http://127.0.0.1:15721`）。
  - token keys（`ANTHROPIC_AUTH_TOKEN`/`ANTHROPIC_API_KEY`/`OPENROUTER_API_KEY`/`OPENAI_API_KEY`）若存在则替换为 `"PROXY_MANAGED"` 占位符（`PROXY_TOKEN_PLACEHOLDER`，proxy.rs:22）；若都不存在则插入 `ANTHROPIC_AUTH_TOKEN = "PROXY_MANAGED"`。
  - `ManagedAccount` 策略：移除所有 token keys，插入 `ANTHROPIC_API_KEY = "PROXY_MANAGED"`。
  - 移除 `CLAUDE_MODEL_OVERRIDE_ENV_KEYS`，插入 takeover model 字段（`ANTHROPIC_DEFAULT_HAIKU/SONNET/OPUS_MODEL` 等，proxy.rs:195-268）。
- **只动 `env`**，其他顶层键（`hooks`/`permissions`/`statusLine`）保留。

**此模式下 settings.json 写什么**：不是"完全不写用户配置"——而是**保留用户配置的整体结构，只把 env 里的凭据/URL/model 字段替换为代理占位符**。用户 hooks/permissions/statusLine 不动。

**此模式下回填保护和 Common Config 是否还生效**：
- **Common Config**：生效。`takeover_live_configs` 写 live 前先 `claude_provider_with_effective_settings`（应用 common config）。`sync_claude_live_from_provider_while_proxy_active`（proxy.rs:304-319）也先构造 effective settings（含 common config）再覆盖 takeover 字段。
- **回填保护（per-provider backfill）**：**不生效**。`switch` 入口（mod.rs:1606-1636）在 `should_hot_switch` 为 true 时走 `hot_switch_provider` 并 `return Ok(SwitchResult::default())`，**跳过 `switch_normal` 的 backfill 分支**。proxy 模式下 provider 切换不做 per-provider backfill，而是通过 `update_live_backup_from_provider_inner`（proxy.rs:1748+）更新 live_backup DB 记录（含 common config），再 `sync_claude_live_from_provider_while_proxy_active` 重写 live 的 takeover env 字段。

**hot_switch_provider 做什么**（proxy.rs:1806-1865）：
- 加 app 级锁。
- 阻断 official provider（proxy.rs:1822-1827）。
- `set_current_provider`（DB + 本地 settings）。
- 若有 live_backup 或 live 已被接管：`update_live_backup_from_provider_inner` + `sync_claude_live_from_provider_while_proxy_active`（重写 takeover env）。
- 通知代理服务器 `set_active_target`。
- **不写 live 的非 env 字段，不调 MCP sync**（mod.rs:1633 注释："Note: No Live config write, no MCP sync"）。

**外围文件是否被管理**：代理接管模式**只接管 `settings.json` 的 env 部分**。`~/.claude.json` 的 mcpServers、`~/.claude/CLAUDE.md`、`~/.claude/skills/`、`~/.claude/projects/` **不由 takeover 模式管理**。这些在 ccs 里由独立服务处理：
- MCP：`McpService::sync_all_enabled(state)`（live.rs:908、944；`mod.rs:1774` 在 `switch_normal` 末尾调用）。**注意：normal 模式每次切换都 sync；proxy/hot_switch 模式不 sync**。
- Skills：`SkillService::sync_to_app`（live.rs:948-952，`sync_current_to_live` 末尾）。
- Prompts：`src-tauri/src/prompt.rs` + `prompt_files.rs`。
- 这些服务与 takeover 正交，在 normal 切换流程末尾统一 sync，不是 takeover 的职责。

### 6. 进入/退出 takeover 时 `cleanup_claude_takeover_placeholders_in_live` 清理什么

**`cleanup_claude_takeover_placeholders_in_live`**（proxy.rs:1562-1591）：
- 读 live `settings.json`。
- 对 env keys `[ANTHROPIC_AUTH_TOKEN, ANTHROPIC_API_KEY, OPENROUTER_API_KEY, OPENAI_API_KEY]`：若值 == `"PROXY_MANAGED"`，删除该 key。
- 对 `ANTHROPIC_BASE_URL`：若是本地代理 URL（`is_local_proxy_url` 判断 `127.0.0.1`/`localhost`/`0.0.0.0`/`[::1]`/`[::]`/`::1`/`::`，proxy.rs:1547-1560），删除。
- `write_claude_live` 写回。
- 清理后通常紧跟 `restore_live_configs`（proxy.rs:1373-1390）：从 DB `proxy_live_backup` 表读出原始 `settings.json` 全文，`write_claude_live` 整体覆盖回去。所以 cleanup 是防御性剥离占位符，**真正的还原是 `proxy_live_backup` 表里的整文件覆盖**。

**进入 takeover 的清理**：进入时不调 cleanup，直接 `takeover_live_configs` 覆写 env 字段。

**退出 takeover 的清理**：`restore_live_configs`（proxy.rs:1373+）→ 从 `proxy_live_backup` 还原整文件 → `cleanup_claude_takeover_placeholders_in_live`（防御）→ `set_live_takeover_active(false)` → `delete_all_live_backups()`。

---

## 关键文件清单

| 文件路径 | 作用 |
|---|---|
| `src-tauri/src/services/provider/live.rs` | backfill、common config deep-merge/remove、`write_live_snapshot`、`write_live_with_common_config`、`strip_common_config_from_live_settings` |
| `src-tauri/src/services/provider/mod.rs:1643-1777` | `switch_normal`：backfill + set_current + write_live_with_common_config + MCP sync |
| `src-tauri/src/services/provider/mod.rs:1600-1640` | `switch` 入口：proxy 模式 hot_switch 分流 |
| `src-tauri/src/services/proxy.rs:140-193` | `apply_claude_takeover_fields_for_provider`：Claude takeover env 字段改写 |
| `src-tauri/src/services/proxy.rs:439-497` | `start_with_takeover`：进入代理接管 |
| `src-tauri/src/services/proxy.rs:1150-1214` | `takeover_live_configs`：各 app 写接管配置 |
| `src-tauri/src/services/proxy.rs:1562-1591` | `cleanup_claude_takeover_placeholders_in_live` |
| `src-tauri/src/services/proxy.rs:1806-1865` | `hot_switch_provider`：proxy 模式下切 provider |
| `src-tauri/src/database/dao/settings.rs:59-136` | Common Config Snippet 的 DB 存取 |
| `src-tauri/src/database/schema.rs:27-43` | `providers` 表（`settings_config`、`meta`、`sort_index`） |
| `src-tauri/src/database/schema.rs:249-255` | `proxy_live_backup` 表（takeover 整文件备份） |

## Caveats / Not Found

- `write_json_file` 的具体实现（是否启用 `preserve_order` feature）未深挖，但对切换语义无影响。
- `sync_live_to_providers`（进入 takeover 时同步 token 到 DB）的具体逻辑未逐行读，但语义上类似 backfill，仅针对 token 字段。
- `CLAUDE_MODEL_OVERRIDE_ENV_KEYS` 的完整列表见 proxy.rs 顶部常量，未全引。
- ccs 前端 `claudeProviderPresets.ts`/`useCommonConfigSnippet.ts` 未逐行读——后端已完整覆盖语义，前端只是 snippet 编辑器 + per-provider `common_config_enabled` 开关 UI。
