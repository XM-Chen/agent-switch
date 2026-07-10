# Design: 一键从本地 ccs 导入 Claude 上游渠道

> 配套：`prd.md`（需求/验收）、`implement.md`（执行计划）、`research/ccs-data-format.md`、`research/agent-switch-direct-provider-path.md`。

## 设计总览

新增一个**后端编排端点** `POST /api/providers/import-ccs`，一次性完成：探测 `~/.cc-switch/config.json` → 解析 ccs provider 列表 → 为每个勾选项建 endpoint（加密存 token）+ 建 direct provider（引用 endpoint_id + meta 记录来源）→ 去重（按 meta.original_id）+ 冲突重命名（按 name）。前端新增"从 ccs 导入"按钮 + 预览对话框，先调探测端点拿列表，用户勾选后再调导入端点。

**不改动现有 direct 切换路径**（`switch` → `enable_direct` → `apply_direct`），导入只产生符合契约的 endpoint+provider 数据，切换复用现有逻辑。

## 架构边界

```
前端（src/）
  ProvidersPage.tsx ── 新增"从 ccs 导入"按钮
  components/providers/ImportCcsDialog.tsx ── 新建：预览+勾选+导入对话框
  lib/api.ts ── 新增 ccsImportApi（detect / import 两个方法）
        │
        │  HTTP
        ▼
后端（src-tauri/src/）
  http/api/providers.rs ── 新增两个路由
    POST /api/providers/import-ccs/detect  → 返回 ccs provider 预览列表（只读）
    POST /api/providers/import-ccs         → 执行导入（批量编排）
  services/importers/ccs.rs ── 新建：ccs 解析 + 映射 + 编排逻辑
  services/importers/mod.rs ── 新建：importers 模块入口
```

**不新增** Tauri command——导入走 HTTP API（与现有 providers/endpoints 一致，前端用 fetch）。**不新增**前端文件系统访问——ccs 文件由后端读取，前端不碰磁盘。

## 数据流与契约

### 1. 探测：`POST /api/providers/import-ccs/detect`

请求体：空（或可选 `{"config_path": "..."}` 覆盖默认路径，便于排错）。

响应体 `DetectResponse`：
```json
{
  "config_path": "~/.cc-switch/config.json",
  "found": true,
  "providers": [
    {
      "original_id": "550e8400-...",
      "name": "DeepSeek",
      "base_url": "https://api.deepseek.com/anthropic",
      "has_api_key": true,
      "model": "deepseek-chat",
      "website_url": "https://deepseek.com",
      "importable": true,
      "conflict": false,
      "imported_name": "DeepSeek",
      "already_imported": false,
      "warning": null
    }
  ]
}
```

字段语义：
- `found=false` 时 `providers` 为空，前端显示"未检测到 ccs 安装"。
- `importable=false` 的原因进 `warning`（如"无 base_url（官方登录 preset）"）。
- `conflict=true` 表示与本地已有 provider 同名，`imported_name` 为加后缀后的名称。
- `already_imported=true` 表示本地已有 `meta.original_id` 匹配的 provider，默认不勾选。

### 2. 导入：`POST /api/providers/import-ccs`

请求体 `ImportRequest`：
```json
{
  "items": [
    { "original_id": "550e8400-...", "imported_name": "DeepSeek" }
  ]
}
```

只传 `original_id`（定位 ccs 哪个 provider）+ `imported_name`（前端预览时确定的最终名称，含冲突后缀）。后端按 `original_id` 重新读 ccs config 取 settingsConfig（不信任前端传 base_url/token，避免前端篡改或漏传）。

响应体 `ImportResponse`：
```json
{
  "created_providers": [{ "original_id": "...", "provider_id": "...", "endpoint_id": "...", "name": "..." }],
  "skipped": [{ "original_id": "...", "reason": "..." }],
  "errors": [{ "original_id": "...", "message": "..." }]
}
```

### 3. 后端编排逻辑（`services/importers/ccs.rs`）

**数据源抽象**：ccs 有两种存储格式，detect/import 统一通过一个"读取 claude provider 列表"的内部函数 `read_ccs_providers() -> Vec<CcsSourceProvider>` 屏蔽差异：
- **新版 SQLite**（`~/.cc-switch/cc-switch.db`）：用 `rusqlite` 只读打开（`OpenFlags::SQLITE_OPEN_READ_ONLY`），`SELECT id,name,settings_config,website_url,category FROM providers WHERE app_type='claude'`。`settings_config` 与旧版 `settingsConfig` 结构一致（Claude Code settings.json 全文）。
- **旧版 config.json**（`~/.cc-switch/config.json`）：反序列化 `CcsConfig`，取 providers map。
- detect/import 都先探 SQLite（存在则用），不存在再探 config.json；两者都不存在 → detect `found=false` / import Err。`CcsSourceProvider` 统一字段 `{ id, name, settings_config: Value, website_url: Option<String>, category: Option<String> }`（config.json 源 category 为 None）。后续 extract_env/比对/建 endpoint+provider 逻辑完全复用，不感知数据源。

```
detect(config_path?):
  1. read_ccs_providers()：先探 SQLite db，再探 config.json（详见数据源抽象）；都不存在 → { found: false, providers: [] }
  2. 解析失败 → Err（前端显示"ccs 数据读取失败"）
  3. 对每个 ccs provider：
     a. 从 settings_config.env 提取 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN / ANTHROPIC_MODEL
     b. base_url 缺失/为空 → importable=false, warning="无 base_url（官方登录渠道，无上游端点，无法导入）"
     c. 查本地 providers（app_type=claude-code）按 meta.original_id 匹配 → already_imported
     d. 查本地 providers 按 name 匹配 → conflict；计算 imported_name（加 (ccs) 后缀，递增）
     e. 组装 DetectItem
  4. 返回 DetectResponse

import(items, state):
  对每个 item（逐项独立，单个失败不影响其它——非全有全无）：
  1. read_ccs_providers() 重新读 ccs，按 original_id 找到 ccs provider（找不到 → skipped, reason="ccs 中不存在该 id"）
  2. 提取 base_url / api_key / model
  3. base_url 为空 → skipped, reason="无 base_url"
  4. 建 endpoint：
     - id = uuid v4
     - name = imported_name（与 provider 同名，便于用户关联）
     - base_url, protocol_type="anthropic", auth_mode="api_key"
     - api_key = ccs env.ANTHROPIC_AUTH_TOKEN（经 encrypt_api_key 加密；None 时 api_key_encrypted=None）
     - account_id = None
     - 调 endpoints::create
  5. 建 provider：
     - id = uuid v4
     - app_type = "claude-code", mode = "direct"
     - name = imported_name
     - settings_config = json!({"endpoint_id": <上面 endpoint id>, "model"?: <ccs model>})
     - category = "custom"
     - meta = json!({"imported_from":"ccs","original_id":<ccs id>,"website_url":<ccs website_url 或 null>})
     - 调 providers::create
  6. 成功 → created_providers；endpoint 建成功但 provider 失败 → 回滚 endpoint（delete）+ errors
```

### 4. 冲突重命名算法

```
fn resolve_unique_name(desired: &str, existing_names: &HashSet<String>) -> String {
  if !existing_names.contains(desired) { return desired.to_string(); }
  let base = format!("{} (ccs)", desired);
  if !existing_names.contains(&base) { return base; }
  for i in 2..=1000 {
    let cand = format!("{} (ccs {})", desired, i);
    if !existing_names.contains(&cand) { return cand; }
  }
  // 兜底：加 uuid 前缀
  format!("{} (ccs {})", desired, uuid::Uuid::new_v4())
}
```

`existing_names` = 本地 `providers.name` where app_type=claude-code ∪ 本次导入已确定的名字（避免批量导入内部撞名）。

### 5. 去重依据

`already_imported` 判定：遍历本地 providers，解析 `meta` JSON，匹配 `meta.imported_from=="ccs" && meta.original_id==<ccs id>`。导入端点不再为已导入项创建（前端默认不勾选，但用户强行勾选时后端二次校验：若 meta.original_id 已存在 → skipped, reason="已导入过"）。

## 兼容性与迁移

- **不动现有表结构**：复用现有 `endpoints` / `providers` 表，无新迁移。
- **不动现有切换路径**：导入数据符合 `DirectSettings` 契约（含 endpoint_id），`apply_direct` 正常消费。
- **`CreateProviderRequest` 不扩展 meta**：保持现有 create 端点语义纯净，导入走专用端点直接构造 `NewProvider`（后端 service 层可直接调 `providers::create` 并传 meta，DAO 层 `NewProvider` 本就有 meta 字段，只是 HTTP create 端点没暴露）。
- **crypto 降级**：`state.crypto` 为 None（Keychain 不可用）时，含 api_key 的项在 import 阶段调 `encrypt_api_key` 会返回 503；detect 阶段可提前探测 crypto 可用性并标注 `warning="系统凭据管理器不可用，含 API Key 的渠道无法导入"`。

## 关键 trade-off

1. **专用端点 vs 复用 create**：选专用端点。代价是多写一个路由+service；收益是 ccs 解析/去重/重命名逻辑封在后端，前端不碰文件系统，且不污染 create 语义。
2. **非事务全有全无 vs 逐项独立**：选逐项独立（单个失败记入 errors，其它继续）。导入是批量用户操作，一个失败回滚全部体验更差；逐项独立让用户看到部分成功 + 失败清单。endpoint+provider 两表在单项内仍原子（provider 失败回滚 endpoint）。
3. **detect 读 ccs，import 再读一次 ccs**：不信任前端传 base_url/token，import 时按 original_id 重新读源文件。代价是两次读文件；收益是防前端篡改/漏传，且 detect 到 import 间 ccs 文件若变化，import 取最新值。
4. **endpoint 与 provider 同名**：便于用户在 endpoints 页和 providers 页关联同一渠道。endpoint 无 name 唯一约束，不冲突。
5. **空 env 项（官方登录）不导入**：agent-switch direct 模式必须有 base_url 才能写 settings.json，官方登录走 OAuth 无 base_url，导入后无法切换生效。标注为不可导入比导入一个废行更好。

## 回滚考虑

- 单项导入失败：endpoint 已建 → `endpoints::delete(endpoint_id)` 回滚，provider 不建。
- 整次导入用户想撤销：导入的 provider/endpoint 可在 UI 手动删除（现有删除能力）。本期不做"批量撤销导入"按钮（meta 已记录 imported_from，后续可加批量删除 imported_from=ccs 的功能）。
- 不动 ccs 源文件，删除导入项不影响 ccs。

## 操作约束

- 导入端点要求 crypto 可用（含 api_key 项）；不可用时返回 503 + 明确提示。
- 导入不激活任何 provider（`is_current` 恒 0），用户导入后自行切换。
- `protocol_type="anthropic"`、`auth_mode="api_key"` 硬编码（ccs 渠道都是 Anthropic 协议 + API Key 鉴权）。
