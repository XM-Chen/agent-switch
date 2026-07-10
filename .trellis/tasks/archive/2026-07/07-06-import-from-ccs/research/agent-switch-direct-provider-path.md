# Research: agent-switch direct provider 完整代码路径（创建→切换→生效）

- **Query**: 摸清 direct 模式 provider 从"创建落库"到"切换生效"的完整代码路径，为 ccs 导入功能提供设计依据
- **Scope**: internal（agent-switch 本仓库工作树）
- **Date**: 2026-07-06

## 结论摘要（先看这里）

**ccs 的 `settingsConfig` 不能原样塞进 `providers.settings_config` 并在切换时生效。** 这是本次研究最关键的结论。

agent-switch 的 direct 模式做了一个**偏离 ccs 的架构决策**：direct provider 的 `settings_config` 不是 Claude Code 的 `settings.json` 原样（不含 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`），而是一个**只含 `endpoint_id` 引用**的自定义子集：`{"endpoint_id": "...", "model": ..., "wire_api": ..., "requires_openai_auth": ...}`。真实的 `base_url` 和 API key（token）在切换时从 `endpoints` 表按 `endpoint_id` 查出，key 还需 AES-256-GCM 解密。ccs 的做法是明文内联，两者数据模型不兼容。

**因此 ccs 导入不能只写 `providers` 一行。** 每导入一个 ccs 项，至少要：(1) 在 `endpoints` 表建一行（含加密后的 api_key），(2) 在 `providers` 表建一行 `mode=direct`、`settings_config={"endpoint_id": <上面那行>}`。若跳过 endpoints、直接把 ccs 明文塞进 settings_config，切换时 `resolve_direct_config` 会因缺少 `endpoint_id` 字段而 JSON 反序列化失败（`DirectSettings.endpoint_id` 是必填、无 `#[serde(default)]`），provider 永远无法激活。

现有 `POST /api/providers` create 端点**不足以完成导入**，因为它只写 providers 表、不碰 endpoints、也不做加密。导入功能需要新的编排逻辑（新端点或后端服务），把 endpoint 创建 + 加密 + provider 创建串起来。

## Findings

### direct provider 生命周期时序

```
创建 (POST /api/providers)
  → providers::create()  ── is_current 恒 0，不激活
                          ── settings_config 原样存字符串（前端传什么存什么）

切换 (POST /api/providers/{id}/switch)
  → switch handler → perform_switch()
     1. providers::get(id)                     查目标
     2. Tool::from_str(app_type).supports_takeover()  校验支持接管
     3. providers::get_current()               记录旧 current（回滚用）
     4. providers::set_current(id)             先设 is_current=1（事务互斥）
     5. 按 mode 分派接管：
          mode=="direct" → tool_takeover::enable_direct(db, tool, data_dir, provider, crypto)
          否则(proxy)    → tool_takeover::enable(db, tool, data_dir)
     6. 接管失败 → 回滚 is_current 到旧 current（或 clear）

生效 (enable_direct → enable_direct_at)
     a. 校验 supports_takeover / app_type 匹配 / mode=="direct"
     b. resolve_direct_config()   解析 settings_config + 查 endpoint + 解密 key
     c. backup_before_write()     备份原工具文件
     d. claude_code::apply_direct(config_dir, &cfg)  写 ~/.claude/settings.json
     e. dao::upsert_state(tool, enabled=1, mode="direct", active_provider_id=id, target=base_url)
```

关键点：**切换 direct provider 确实会调用 `apply_direct`**（经由 `enable_direct` → `enable_direct_at` → `claude_code::apply_direct`），路径 `src-tauri/src/http/api/providers.rs:231`。

### settings_config 的精确 JSON 契约（direct 模式）

由 `DirectSettings` 结构定义（`src-tauri/src/services/tool_takeover/mod.rs:129-138`）：

```json
{
  "endpoint_id": "<必填，引用 endpoints 表某行 id>",
  "model": "<可选，写入 env.ANTHROPIC_MODEL>",
  "wire_api": "<可选，Codex 专属，缺省 responses>",
  "requires_openai_auth": true
}
```

- `endpoint_id`：**必填**（无 `#[serde(default)]`），缺失则 `serde_json::from_str` 反序列化失败 → 切换报错。
- 其余字段均 `#[serde(default)]` 可选。
- **不含** `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`——这两个值在切换时才动态组装（base_url 取自 endpoint.base_url，token 取自 endpoint 解密后的 api_key）。

对比 proxy 模式：`settings_config` 通常是 `{}`（回填行/默认代理都用空对象，见 `providers.rs:375`）。

### apply_direct 消费的 DirectConfig 来源

`DirectConfig`（`mod.rs:112-123`）由 `resolve_direct_config`（`mod.rs:264-305`）组装：

| DirectConfig 字段 | 来源 |
|---|---|
| `provider_id` | `provider.id`（Codex 用作 model_provider 表名）|
| `base_url` | `endpoint.base_url`（**从 endpoints 表查，非 settings_config**）|
| `api_key` | `endpoint.api_key_encrypted` 经 `crypto.decrypt(encrypted, endpoint.id.as_bytes())` 解密，再从 JSON 取 `api_key` 字段（明文，仅写文件不落 DB）|
| `model` | `settings.model`（来自 settings_config）|
| `wire_api` | `settings.wire_api`（来自 settings_config）|
| `requires_openai_auth` | `settings.requires_openai_auth`（来自 settings_config）|

`claude_code::apply_direct`（`claude_code.rs:92-124`）把 `cfg.base_url` 写入 `env.ANTHROPIC_BASE_URL`、`cfg.api_key` 写入 `env.ANTHROPIC_AUTH_TOKEN`、`cfg.model`（若有）写入 `env.ANTHROPIC_MODEL`，**合并写入**（保留 settings.json 其它顶层键和 env 内其它键），原子写。

解密依赖：crypto 服务不可用 → 报"加密服务不可用"→ switch 返回 503；endpoint 不存在 → 报"端点不存在"→ 500。两者都触发 is_current 回滚。

### tool_takeover 与 providers 的关系

**是两套表，但切换 provider 会同时驱动两者——不是同一个动作，而是 provider 切换编排里包含了 tool_takeover 的写入。**

- `providers` 表：ccs 式"可切换单元"，UI 选哪个、当前激活哪个（`is_current`）。
- `tool_takeover` 表：记录每个工具的接管状态（enabled / mode / active_provider_id / last_target 等）。

切换一个 direct provider 时**会写 tool_takeover 表**：`enable_direct` 末尾调 `dao::upsert_state(tool, enabled=1, mode="direct", active_provider_id=provider.id, target=base_url)`（`mod.rs:245`）。即 `tool_takeover.active_provider_id` 反向指回被激活的 provider，`tool_takeover.mode` 记录当前是 direct 还是 proxy。

反向耦合：删除 current direct provider 时（`providers.rs:192-201`），若 `tool_takeover.active_provider_id` 指向被删 provider，会把该 tool 的 takeover 重置为 `mode=proxy` + 清空 `active_provider_id`，避免悬挂引用。

回填逻辑（`backfill_from_takeover`, `providers.rs:335`）：存量 `tool_takeover.enabled=1` 但 providers 空的用户，一次性造 proxy provider 桥接。说明历史上 tool_takeover 先存在，providers 是后加的切换面。

### 现有 create API 请求体结构

`CreateProviderRequest`（`providers.rs:59-68`）：

```json
{
  "app_type": "claude-code" | "codex",   // 必填
  "name": "string",                        // 必填
  "mode": "proxy" | "direct",              // 可选，缺省 proxy
  "settings_config": { ... },              // 必填，任意 JSON Value
  "category": "string | null",             // 可选
  "notes": "string | null"                 // 可选
}
```

后端处理（`providers.rs:136-157`）：`id` 用 uuid 自动生成；`sort_index` 自动 `next_sort_index`（MAX+1）；`meta` 恒 `"{}"`；`is_current` 恒 0（create 内 SQL 硬编码 `VALUES (..., 0, ...)`, `providers.rs:154`）。`settings_config` 直接 `.to_string()` 原样存字符串，后端**不校验其内部结构**（校验推迟到切换时）。

前端 TS 契约（`src/lib/api.ts:118-131`）：`CreateProviderBody` 与后端一致，`settings_config: unknown`。

### 前端 ProviderForm 如何组装 settings_config

`src/components/providers/ProviderForm.tsx`：

- `settings_config` 用一个 **JSON textarea 手填**（`settingsText` state），提交前 `JSON.parse` 校验合法性（`ProviderForm.tsx:52-58`）。
- **没有字段级映射**——direct 模式的 endpoint_id 需用户在 JSON 里手写（placeholder `{"endpoint_id":"..."}`, `ProviderForm.tsx:168`；注释明说 "endpoint_id 选择器留 P1 后深度绑定，本期直接在 JSON 中手填"，`ProviderForm.tsx:25`）。
- 即前端目前**不做任何 ccs 风格的 settings.json 组装**，纯透传用户输入的 JSON 文本。

### 是否需要新端点的判断

**需要新的导入编排（新端点或后端 service），不能只复用现有 create 端点。** 理由：

1. ccs 项含明文 base_url + api_key。要落成能切换生效的 direct provider，必须先在 `endpoints` 表建行并加密 api_key（`crypto.encrypt(json!({"api_key": ...}), endpoint.id.as_bytes())`，参考 `mod.rs:606-607` 测试里的加密方式），再让 provider 的 settings_config 引用该 endpoint_id。
2. 现有 `POST /api/providers` 只写 providers 表，不创建 endpoint、不加密。单靠它导入，切换时必然 `resolve_direct_config` 失败。
3. 导入通常是批量、需去重/幂等（endpoint 复用判断），编排复杂度高于单条 create。

可选方案（供设计参考，未决）：
- A. 新增 `POST /api/providers/import`，后端一次性建 endpoint + provider（原子）。
- B. 让导入分两步：先调现有 endpoint 创建 API（若存在），再调 create providers。需确认 endpoints 是否已有 HTTP create 端点（本次未查 endpoints 的 HTTP API，见 Caveats）。

## Related Specs

- 任务 PRD：`.trellis/tasks/07-06-import-from-ccs/prd.md`（本次未通读，建议对照）
- `mod.rs:126-128` / `mod.rs:262` 注释明确写道 direct 用 endpoint_id 而非内联明文 key，"偏离 ccs 明文做法，见任务 PRD 决策"——说明 PRD 里已有相关决策记录。

## Caveats / Not Found

- **未查 endpoints 的 HTTP API**（`src-tauri/src/http/api/` 下是否有 endpoints create 端点、请求体结构）。若导入方案要复用 endpoints 创建能力，需补查 `endpoints` DAO（`src-tauri/src/db/dao/endpoints.rs`）和其 HTTP 路由。这是设计导入编排前的必要下一步。
- **未查 crypto 服务如何在 HTTP handler 中获取**（`state.crypto` 的初始化/可用性条件）。导入若在后端加密，需确认 crypto service 在导入端点上下文可用。
- **endpoints 表的 api_key 加密格式**：从测试推断为 `encrypt(serde_json::to_vec(&json!({"api_key": <明文>})), endpoint.id.as_bytes())`（`mod.rs:606-607`），解密后取 JSON 的 `api_key` 字段（`mod.rs:289-295`）。AAD 是 `endpoint.id`。导入建 endpoint 时必须遵循同一格式，否则切换解密失败。
- ProvidersPage.tsx 本次未读（任务参考起点列了它但未深入），前端导入 UI 如何接线需另查。
- Codex direct 路径（`codex::apply_direct`）本次只看到 mod.rs 分派，未读 `codex.rs` 具体写法；claude-code 路径已完整确认。
