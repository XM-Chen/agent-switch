# 设计 — 导入导出与设置

> 配套 `prd.md`。本文件写技术设计:容器格式、加密契约、导出/导入数据流、冲突 merge 算法、API、依赖与取舍。

## 1. 架构与分层

复用现有应用层分层:

```text
HTTP handler (http/api/settings.rs 扩展)   参数校验 + 响应组装
        │
服务层 (services/portability/)             导出装包、导入解包、加密、冲突 merge
        │
DAO 层 (各表 dao + 新增 import 批量写)       SQL 读写
        │
SQLite + Keychain(主密钥) + CryptoService
```

### 服务层模块拆分

```text
src-tauri/src/services/portability/
├── mod.rs        编排:export(mode) / import(package, password?, conflict_mode)
├── package.rs    ExportPackage 容器结构、序列化/反序列化、版本校验
├── crypto_box.rs 双密钥加密:主密钥模式 + Argon2id 密码模式;gzip 压缩
├── collect.rs    导出:从各表收集数据 → payload(脱敏模式剔除凭据列)
└── apply.rs      导入:replace / merge 策略,事务写入,冲突 upsert
```

> 现有 `services/` 含扁平文件与 `tool_takeover/` 子目录。本任务涉及多文件协作(装包+加密+收集+应用),采用子模块目录,符合既有约定。

## 2. 容器格式(package.rs)

```rust
#[derive(Serialize, Deserialize)]
pub struct ExportPackage {
    pub format_version: u32,        // 当前 = 1
    pub mode: String,               // "full_backup" | "portable"
    pub algo: String,               // "AES-256-GCM"
    pub kdf: String,                // "none" | "argon2id"
    pub kdf_params: Option<KdfParams>, // 仅 argon2id
    pub nonce: String,              // base64(12 bytes)
    pub created_at: String,         // ISO 8601
    pub app_version: String,        // env!("CARGO_PKG_VERSION")
    pub ciphertext: String,         // base64(AES-GCM(gzip(payload_json)))
}

#[derive(Serialize, Deserialize)]
pub struct KdfParams {
    pub salt: String,    // base64(16 bytes)
    pub m_cost: u32,     // Argon2id 内存 (KiB),默认 19456 (19 MiB)
    pub t_cost: u32,     // 迭代,默认 2
    pub p_cost: u32,     // 并行,默认 1
}
```

明文 payload:

```rust
#[derive(Serialize, Deserialize)]
pub struct Payload {
    pub accounts: Vec<AccountExport>,       // 脱敏模式 credentials=None
    pub endpoints: Vec<EndpointExport>,     // 脱敏模式 api_key=None
    pub endpoint_models: Vec<ModelExport>,  // custom + synced
    pub model_aliases: Vec<AliasExport>,
    pub route_settings: Vec<RouteSettingExport>,
    pub tool_takeover: Vec<ToolTakeoverExport>, // 仅 enabled 标记,导入后强制关闭
    pub ui_settings: Vec<(String, String)>, // app_metadata 中可迁移偏好项（白名单，见下）
    // 不含 request_logs / model_locks / 测试数据
}
```

- AAD 用固定串 `"agent-switch-export-v1"`(包级,非记录级)。
- `format_version` 不匹配 → 拒绝导入并提示版本。

### ui_settings 白名单

`ui_settings` 只导出**偏好类** app_metadata 键，排除本机运行状态快照：

| key | 导出 | 理由 |
|---|---|---|
| `auto_model_refresh_enabled` | ✅ | 用户偏好，跨机器有意义 |
| `last_model_sync_at` | ❌ | 本机运行状态，迁移无意义 |
| `last_model_sync_error` | ❌ | 本机运行状态，迁移无意义 |

> 实现用**白名单常量**（`const PORTABLE_METADATA_KEYS: &[&str] = &["auto_model_refresh_enabled"]`），而非黑名单。后续新增 app_metadata 键默认不导出，需显式加入白名单，避免误导出新的本机状态键。full_backup 模式同样只导出白名单偏好键（本机状态对同机恢复也无意义）。

## 3. 加密契约(crypto_box.rs)

### 3.1 完整备份(主密钥模式)
- key = `keychain::load_master_key()`(32 字节);None → 503。
- `kdf = "none"`,无 kdf_params。
- 复用 `CryptoService::encrypt/decrypt` 思路,但 AAD 用包级常量。

### 3.2 脱敏迁移(密码模式)
- 用户输入导出密码 → Argon2id(salt 随机 16 字节,默认 m=19MiB/t=2/p=1)→ 32 字节 key。
- `kdf = "argon2id"`,kdf_params 写入包(导入时复现派生)。
- 弱密码检测:长度 < 8 或全同字符类 → 返回 warning 字段(不阻止)。

### 3.3 压缩
- payload JSON → flate2 gzip → 加密。解密 → gunzip → 反序列化。

## 4. 导出数据流(collect.rs + mod.rs::export)

```text
export(mode):
  1. 收集各表 → Payload
     - full_backup: 凭据列原样(加密 BLOB base64,导入时连同主密钥环境恢复)
     - portable:    凭据列置 None;剔除 request_logs
  2. serde_json::to_vec(payload) → gzip
  3. 加密(主密钥 or 密码派生)→ ExportPackage
  4. serde_json::to_string_pretty(package) → 返回文本(前端下载)
```

> full_backup 凭据装包方式:直接装入**当前 DB 中的加密 BLOB**(已是主密钥加密)。导入到同一主密钥环境可直接写回;跨环境无法解密(符合父 PRD「绑定本机/同凭据环境」)。无需先解密再重加密,减少明文暴露面。

## 5. 导入数据流(apply.rs + mod.rs::import)

```text
import(package, password?, conflict_mode):
  1. 解析 ExportPackage,校验 format_version
  2. 解密:
     - kdf=none   → 主密钥;失败 → 可读错误
     - kdf=argon2id → 密码派生;失败 → "密码错误或包损坏"
  3. gunzip → 反序列化 Payload
  4. full_backup 前:自动本地 DB 备份(复制 sqlite 文件到 data_dir/backups/db/)
  5. BEGIN TRANSACTION
     - 按 D1 矩阵 replace / merge 各表
     - tool_takeover: 全部 enabled=0
     - 跳过 request_logs / 测试数据
  6. COMMIT;失败 → ROLLBACK + 可读错误
```

### 5.1 merge 匹配键(脱敏导入)

| 表 | 匹配键 | 命中 | 未命中 |
|---|---|---|---|
| accounts | name + account_type + platform | 更新非敏感字段 | 新增(新 id) |
| endpoints | name + base_url + protocol_type | 更新非敏感字段(不动 api_key) | 新增 |
| endpoint_models | endpoint_id + model_name | upsert | 新增 |
| model_aliases | scope_type + scope_id + alias_name | upsert | 新增 |
| route_settings | id (claude-code/codex/v1) | upsert | 新增 |

- 新增账号/端点用新 UUID,避免 id 冲突;导出内原 id 仅用于 payload 内部关联(endpoint→account)重映射。
- id 重映射:导入时建 `old_id → new_id` 映射表,endpoint.account_id、alias.target_endpoint_id、model.endpoint_id 按映射改写。

### 5.2 replace(完整备份导入)
- 各表先 DELETE 再按包内容 INSERT(保留原 id,因为是同环境恢复)。
- 在事务内,失败回滚。

## 6. DAO 扩展

- 各表 dao 增 `insert_raw`(导入用,保留 id)或复用现有 create(merge 用新 id)。
- 新增 `db/dao/import_helpers.rs` 或在 portability 服务内直接用 Connection 批量写(事务内)。
- 本地 DB 备份:复制 sqlite 文件(参考 tool_takeover backup 的文件复制 helper)。

## 7. HTTP API(settings.rs 扩展)

```text
POST /api/settings/export
     body { mode: "full_backup"|"portable", password?: string }
     → 200 { package: string(JSON文本), warnings?: [string] }
       full_backup 主密钥不可用 → 503
       portable 缺密码 → 400

POST /api/settings/import
     body { package: string, password?: string, conflict_mode?: "auto" }
     → 200 { imported: {accounts:n, endpoints:n, ...}, warnings?: [] }
       解密失败 → 400 可读错误
       version 不匹配 → 400

GET  /api/settings/export/preview   (可选,本期可省略)
```

- handler 只校验参数,调用 portability 服务。
- 响应不回显任何明文凭据。

## 8. 前端(SettingsPage 扩展 + lib/api.ts)

- `lib/api.ts` 增 `portabilityApi`:`exportConfig(mode, password?)`、`importConfig(package, password?)`。
- `SettingsPage` 增「配置导入导出」卡片:
  - 完整备份导出按钮(直接下载,提示绑定本机)。
  - 脱敏导出:密码输入 + 弱密码提示 + 导出按钮。
  - 导入:文件选择(读取文本)+ 密码输入(脱敏包需要)+ 导入按钮 + 冲突提示。
  - 风险提示文案(中文):凭据绑定本机、跨机器需重录、覆盖/合并、接管导入后关闭。
- 导出下载:前端用 `Blob` + `a[download]` 触发保存,文件名含 mode + 时间戳。

## 9. 依赖

- 新增 `argon2 = "0.5"`(脱敏包密码派生)。
- 新增 `flate2 = "1"`(gzip 压缩)。
- 复用:`aes-gcm`、`base64`、`serde_json`、`rusqlite` 事务、`uuid`、`time`。

## 10. 关键取舍

1. **双密钥而非单一主密钥**:完整备份用主密钥(绑定本机,父 PRD 要求),脱敏迁移用密码派生(实现可跨机器解密)。这解决了「强制加密」与「可迁移」的矛盾,是父 PRD 自述方向的最优落地。
2. **full_backup 装入加密 BLOB 而非解密重装**:减少明文凭据暴露面,且天然实现「绑定主密钥环境」语义。
3. **脱敏导入用新 UUID + id 重映射**:避免与本机已有记录 id 冲突,同时保持包内 endpoint→account、alias→endpoint 关联完整。
4. **导入事务化**:任一步失败回滚,杜绝半成品配置(配置类操作的安全底线)。
5. **接管状态强制关闭**:继承父 PRD 已定语义,导入绝不静默污染本机工具配置。
6. **gzip 压缩在加密前**:端点/模型多时显著减小包体积;加密后数据高熵不可再压。

## 11. 与其它子任务衔接

- 依赖全部 7 个前置子任务的表结构(accounts/endpoints/models/aliases/route_settings/tool_takeover)。
- `request_logs`(routing-failover-core 建)与测试数据(chain-testing-debugger)显式排除导出。
- 是 MVP 最后一块:完成后父任务 8/8,可整体集成验收。
