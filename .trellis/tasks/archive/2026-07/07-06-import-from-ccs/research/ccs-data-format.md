# Research: ccs 数据格式与边界情况

- **Query**: 摸清 cc-switch (ccs) 项目里 Claude provider 的完整数据格式和边界情况，为导入功能提供设计依据
- **Scope**: internal (ref-cc-switch/tauri-migration 分支)
- **Date**: 2026-07-06

> 说明：本报告聚焦 **`ref-cc-switch/tauri-migration` 分支**（用户指定研究范围）。该分支是 ccs 早期 Tauri 迁移版本，只支持 Claude provider，纯文件系统存储（`~/.cc-switch/config.json` + `~/.claude/settings-<name>.json`），无数据库、无多应用、无代理。末尾的「与 main 分支差异」节作为前瞻参考，便于判断导入功能的数据模型取舍。

---

## Findings

### Files Found

| File Path | Description |
|---|---|
| `ref-cc-switch/tauri-migration:src-tauri/src/provider.rs` | `Provider` / `ProviderManager` 结构体 + 增删改查切换逻辑 |
| `ref-cc-switch/tauri-migration:src-tauri/src/config.rs` | 路径计算、`sanitize_provider_name`、JSON 读写、`import_current_config_as_default` |
| `ref-cc-switch/tauri-migration:src-tauri/src/store.rs` | 全局 `AppState`，加载/保存 `ProviderManager` |
| `ref-cc-switch/tauri-migration:src-tauri/src/commands.rs` | Tauri command 层，含 `import_default_config`、`switch_provider` |
| `ref-cc-switch/tauri-migration:src-tauri/src/lib.rs` | 启动时自动导入逻辑（providers 为空且 `settings.json` 存在时） |
| `ref-cc-switch/tauri-migration:src/types/index.ts` | 前端 `Provider` / `AppConfig` 类型定义 |
| `ref-cc-switch/tauri-migration:src/App.tsx` | `generateId = crypto.randomUUID()`、`handleAutoImportDefault` |
| `ref-cc-switch/tauri-migration:src/config/providerPresets.ts` | 预设供应商模板（含 `isOfficial` 标记） |
| `ref-cc-switch/tauri-migration:src/components/ProviderForm.tsx` | 添加/编辑表单，含「禁止签名」开关、API Key 自动填充 |
| `ref-cc-switch/tauri-migration:src/utils/providerConfigUtils.ts` | settingsConfig 解析工具（coAuthored、websiteUrl 提取、apiKey 读写） |

---

### 1. `~/.cc-switch/config.json` 完整结构

对应后端 `ProviderManager`（`provider.rs:36-46`）与前端的 `AppConfig`（`types/index.ts`）。

**顶层只有两个字段，没有任何 `version` / `schema` / `category` / `meta` 包裹：**

```jsonc
{
  "providers": {
    "<provider-id>": {
      "id": "uuid-或-default",
      "name": "用户可读名称",
      "settingsConfig": { /* 完整 Claude Code settings.json 内容，见第 2 节 */ },
      "websiteUrl": "https://..."  // 可选，serde skip_serializing_if = Option::is_none
    }
    // ... 更多 provider
  },
  "current": "<provider-id 或 空字符串>"
}
```

要点：
- `providers` 是 **`HashMap<String, Provider>`**（Rust）/ **`Record<string, Provider>`**（TS），key 就是 `provider.id`，是扁平 map，**不是数组**。
- `current` 是一个字符串：要么是某个 provider 的 id，要么是空字符串 `""`（表示无当前 provider）。`ProviderManager::default()` 把 `current` 初始化为 `String::new()`（`provider.rs:43-45`）。
- **`settingsConfig` 是内嵌在 config.json 里的**——`add_provider` / `update_provider` 既把 `settings_config` 写进 `config.json` 的 `providers[id].settingsConfig`，又同时把它另存为 `~/.claude/settings-<name>.json`（`provider.rs:55-72`）。即同一份配置存两份：config.json 内嵌 + 独立文件副本。
- `Provider` 字段就 4 个：`id`、`name`、`settingsConfig`、`websiteUrl?`。**没有 `category`、`createdAt`、`sortIndex`、`notes`、`icon`、`meta` 等任何字段**（这些在 tauri-migration 分支不存在）。

### 2. `~/.claude/settings-<name>.json`（即 `Provider.settingsConfig`）典型内容

由 `get_provider_config_path` 生成的文件，内容就是 Claude Code 的 `settings.json` 全文。`providerPresets.ts` 给出的真实样例：

**样例 A — 第三方 API（DeepSeek v3.1）：**
```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
    "ANTHROPIC_AUTH_TOKEN": "sk-your-api-key-here",
    "ANTHROPIC_MODEL": "deepseek-chat",
    "ANTHROPIC_SMALL_FAST_MODEL": "deepseek-chat"
  }
}
```

**样例 B — 官方登录（Claude官方登录，`isOfficial: true`）：**
```json
{
  "env": {}
}
```

**样例 C — 含「禁止签名」开关的完整形态（来自 `ProviderForm` 的 toggle 与 `providerConfigUtils.updateCoAuthoredSetting`）：**
```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "https://api.example.com",
    "ANTHROPIC_AUTH_TOKEN": "sk-xxx"
  },
  "includeCoAuthoredBy": false
}
```

常见键（来自 presets + utils 汇总）：
- `env.ANTHROPIC_BASE_URL` — 第三方 API 端点
- `env.ANTHROPIC_AUTH_TOKEN` — API Key
- `env.ANTHROPIC_MODEL` — 主模型（部分 preset 用，如 DeepSeek、Kimi）
- `env.ANTHROPIC_SMALL_FAST_MODEL` — 小快模型（部分 preset 用）
- `includeCoAuthoredBy` — `false` 表示禁止 Claude Code 签名（顶层键，不在 env 里）
- `env: {}` — 官方登录场景空 env（走 OAuth，不需要 token）

工具函数行为（`providerConfigUtils.ts`）：
- `extractWebsiteUrl`：从 `env.ANTHROPIC_BASE_URL` 去掉 `api.` 前缀生成官网地址。
- `getApiKeyFromConfig` / `hasApiKeyField`：只读 `env.ANTHROPIC_AUTH_TOKEN`。
- `setApiKeyInConfig`：默认不新增缺失字段，仅当 `createIfMissing: true`（即用户选了 preset）时才创建 `env` / `ANTHROPIC_AUTH_TOKEN`。
- `updateCoAuthoredSetting`：增删顶层 `includeCoAuthoredBy` 字段。

### 3. Provider id 生成规则

**两套来源，并存：**

1. **前端新建 provider** → `crypto.randomUUID()`（`App.tsx:95-97`）。即标准 UUID v4 字符串，如 `550e8400-e29b-41d4-a716-446655440000`。前端 `handleAddProvider` 把它塞进 `Provider.id` 再传给后端 `add_provider`（`App.tsx:100-105`）。

2. **后端「自动导入现有 Claude Code 配置」** → 硬编码字符串 `"default"`（`lib.rs:64-70`、`commands.rs:import_default_config`、`config.rs:import_current_config_as_default`）。即首次启动若 providers 为空且 `~/.claude/settings.json` 存在，会创建一个 `id = "default"`、`name = "default"` 的 provider，并把 `current` 设为 `"default"`。

**注意**：`add_provider` / `update_provider` 不校验 id 唯一性或格式，直接 `providers.insert(id, provider)`。id 是否重复由前端 `generateId` 保证（UUID 碰撞概率忽略）。`with_id` 构造器也不做任何处理。

### 4. `sanitize_provider_name` 规则（影响 `settings-<name>.json` 文件名）

源码（`config.rs:36-44`）：

```rust
pub fn sanitize_provider_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}
```

规则：
- 把 9 个 Windows/POSIX 文件名非法字符 `< > : " / \ | ? *` 全部替换成 `-`。
- 整体转 **小写**。
- **不处理**空格、中文、emoji、`.`、`+`、`(` 等——这些原样保留。
- **不做唯一性去重**：如果两个 provider 的 name sanitize 后相同（如 `My/Provider` 和 `my:provider` 都变成 `my-provider`），会生成**同名的 `settings-my-provider.json`**，后写入的覆盖先写入的（`get_provider_config_path` 只看 name，不看 id）。
- 文件名匹配：`settings-<sanitized-name>.json`，路径为 `~/.claude/settings-<sanitized>.json`（`config.rs:48-53`）。**注意：文件名由 `name` 决定，不由 `id` 决定**——所以 `id` 和文件名是解耦的；切换/编辑时若 name 变了，会先删旧文件再写新文件（`provider.rs:update_provider` 里的 `delete_file(&old_config_path)` 逻辑，`provider.rs:84-91`）。

### 5. 是否有「分组/分类」概念

**tauri-migration 分支：没有。**

- `Provider` 结构体没有 `category` / `type` / `group` 字段（`provider.rs:10-22`）。
- `ProviderManager` 是扁平 `HashMap`，无任何分组容器。
- 唯一的分类痕迹在 `providerPresets.ts` 的 **preset 模板**上：`ProviderPreset` 接口有 `isOfficial?: boolean`（`providerPresets.ts:3`），用于 UI 区分官方登录 preset（如「Claude官方登录」`isOfficial: true`，API Key 输入框禁用）与第三方 API preset。
- 这只是**预设模板的展示标记**，**不会写入 `config.json`**——`ProviderForm.applyPreset` 把 preset 转成 `settingsConfig` 后，`isOfficial` 字段就丢弃了，落盘的 `Provider` 里没有任何分类信息。

**前瞻参考（main 分支，超出本次研究范围但供决策）**：`main` 分支的 `Provider` 增加了 `category: Option<String>` 字段（`main:src-tauri/src/provider.rs:22`），DAO 里也存 `category`（`main:src-tauri/src/database/dao/providers.rs`），出现 `"official"`、`"omo"`、`"omo-slim"` 等取值。即 main 分支才引入了真正的分类概念，且存储迁移到了 SQLite。tauri-migration 分支不存在 `third_party` / `aggregator` 这种分组。

### 6. 边界情况清单（ccs 的实际处理）

| 边界情况 | ccs 处理方式 | 出处 |
|---|---|---|
| **config.json 不存在** | `ProviderManager::load_from_file` 检测 `!path.exists()` → 返回 `Self::default()`（空 providers map + `current = ""`），记 `log::info!("配置文件不存在，创建新的供应商管理器")` | `provider.rs:48-53` |
| **config.json 存在但解析失败** | `store.rs:AppState::new` 用 `unwrap_or_else` 捕获错误，`log::warn!("加载配置失败: {}, 使用默认配置", e)` 后回退到默认空管理器——**原损坏文件不被修复，下次启动仍会失败** | `store.rs:9-17` |
| **providers 为空** | 启动时若 `manager.providers.is_empty()` 且 `~/.claude/settings.json` 存在 → 自动导入为 `id="default"` 的 provider，并设 `current="default"`，再 `save()` | `lib.rs:55-77`；前端 `App.tsx:82-85` 也会在 `loadProviders` 后检测空并调用 `handleAutoImportDefault` |
| **`current` 为空字符串** | `switch_provider` 里 `if settings_path.exists() && !self.current.is_empty()` 才备份当前；空 current 时直接跳过备份，把目标 provider 配置复制到 settings.json | `provider.rs:switch_provider:135-145` |
| **某 provider 的 `settings-<name>.json` 文件缺失** | `switch_provider` 显式检查 `!provider_config_path.exists()` → 返回 `Err("供应商配置文件不存在: <path>")`，**拒绝切换**；`delete_provider` 里 `delete_file` 对不存在文件也会 `Err`，但 `update_provider` 里改名时用 `delete_file(&old).ok()` 忽略删除错误 | `provider.rs:112-117`、`provider.rs:91`、`config.rs:delete_file:101-106` |
| **删除当前正在使用的 provider** | `delete_provider` 首行 `if self.current == provider_id { return Err("不能删除当前正在使用的供应商") }`，**硬拒绝** | `provider.rs:74-76` |
| **切换到不存在的 provider id** | `switch_provider` 用 `ok_or_else` 返回 `Err("供应商不存在: <id>")` | `provider.rs:107-109` |
| **更新不存在的 provider** | `update_provider` 检测 `!self.providers.contains_key(&id)` → `Err("供应商不存在: <id>")` | `provider.rs:80-83` |
| **name 改名导致 sanitize 冲突** | 不检测冲突；删旧 `settings-<old-name>.json`（`.ok()` 忽略失败），写新 `settings-<new-name>.json`，若新名与其他 provider 撞名则**静默覆盖对方文件** | `provider.rs:84-96` |
| **settingsConfig 非法 JSON** | 后端不校验（`serde_json::Value` 接受任意 JSON）；前端 `ProviderForm.handleSubmit` 用 `JSON.parse` 捕获异常，提示「配置JSON格式错误，请检查语法」 | `ProviderForm.tsx:68-78` |
| **官方登录 preset 的空 env** | `isOfficial` preset 的 API Key 输入框 `disabled`，placeholder 提示「官方登录无需填写 API Key，直接保存即可」；`hasApiKeyField` 返回 false 时不显示 API Key 框 | `ProviderForm.tsx`、`providerConfigUtils.hasApiKeyField` |
| **`import_default_config` 重复调用** | `commands.rs:import_default_config` 先检查 `providers.contains_key("default")`，已存在则直接 `Ok(true)` 不重复导入 | `commands.rs:import_default_config:147-155` |

### Related Specs

无直接相关 spec（本仓库 `.trellis/spec/` 下未找到 ccs 数据模型相关文档；当前任务 `07-06-import-from-ccs` 的 PRD/notes 未在 research 阶段读取，由主 agent 决定是否后续 `update-spec`）。

---

## Caveats / Not Found

1. **无测试 fixtures / 样例 config.json**：tauri-migration 分支仓库里没有 `fixtures/`、`*.test.json`、`mock/` 等真实样例文件（`git ls-tree` 确认）。第 2 节的样例来自 `providerPresets.ts` 与 `ProviderForm` 的运行时行为，是「预设模板」而非「落盘样本」。真实 `settings-<name>.json` 内容 = 用户填入的 settingsConfig 全文，结构同上。
2. **`includeCoAuthoredBy` 是否进 config.json**：会进。它是 `settingsConfig` 顶层键，跟着 `settingsConfig` 一起被序列化进 `config.json` 的 `providers[id].settingsConfig`，也同时写进 `settings-<name>.json`。
3. **「分组/分类」的 main 分支差异**：用户问题里提到的 `third_party` / `aggregator` 分组在 tauri-migration 分支**不存在**；这些是 main 分支后续演进（`category` 字段 + SQLite + `official`/`omo` 等取值）。若导入功能要以 tauri-migration 为准，**不应引入 category 字段**；若要兼容 main 分支导出的 config.json，则需要处理 `category` 等额外字段（serde 默认会忽略未知字段，但导入时是否保留这些元数据需主 agent 决策）。
4. **id 与文件名解耦**：`settings-<name>.json` 文件名由 `sanitize(name)` 决定，与 `id` 无关。导入时若直接复制 ccs 的 `config.json`，需要同时为每个 provider 重建对应的 `settings-<name>.json`（ccs 是 `add_provider` 时内嵌+副本双写）。这是导入功能必须复刻的关键行为。
5. **`~/.claude/settings.json` vs `~/.claude/claude.json` 兼容**：`get_claude_settings_path` 优先返回 `settings.json`，不存在但存在旧版 `claude.json` 时返回 `claude.json`，否则默认回落 `settings.json`（`config.rs:13-25`）。导入/切换逻辑都走这个函数，自动兼容旧命名。

---

## 结论摘要（5-10 行）

ccs (tauri-migration) 的 `~/.cc-switch/config.json` 是扁平两字段结构：`providers`（id→Provider 的 map）+ `current`（字符串 id 或空串），无 version/category 包裹；Provider 仅 4 字段 `id/name/settingsConfig/websiteUrl?`，`settingsConfig` 内嵌完整 Claude Code `settings.json` 全文（典型键：`env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`env.ANTHROPIC_MODEL`、`env.ANTHROPIC_SMALL_FAST_MODEL`、顶层 `includeCoAuthoredBy`）。id 由前端 `crypto.randomUUID()` 生成，唯独「自动导入现有配置」用硬编码 `"default"`。`sanitize_provider_name` 把 9 个非法字符替换成 `-` 再转小写，决定 `settings-<name>.json` 文件名——**文件名由 name 决定、与 id 解耦**，且不做去重（同名静默覆盖）。该分支**无任何分组/分类概念**，仅 preset 模板上有 `isOfficial` 展示标记且不落盘；`third_party`/`aggregator`/`category` 是 main 分支后续演进，不在本次范围。边界处理要点：config.json 缺失/解析失败均回退空管理器（不修复损坏文件）、providers 为空时自动导入 default、删除当前 provider 硬拒绝、切换/更新前校验存在性、provider 配置文件缺失时拒绝切换。导入功能若要复刻，关键点是「config.json 内嵌 + `settings-<name>.json` 副本双写」与「name→文件名 sanitize」这两条行为。
