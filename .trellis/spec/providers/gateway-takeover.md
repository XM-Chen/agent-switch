# 本地网关与模块接管

## 1. 适用范围 / 触发条件

修改以下任一内容前必须遵守本文：

- `proxy_config` schema、迁移或 DAO；
- 本地网关启动/停止命令；
- 七模块接管状态、`route_mode` 或前后端 wire 类型；
- live 配置接管、异常恢复、退出清理或精确快照；
- 七模块外部配置检测、冲突 generation、accept/reject 或 managed-write 抑制；
- failover/profile 更新应用级代理配置。

七模块固定为 `AppType::as_str()` 的规范值：`claude`、`claude-desktop`、`codex`、`gemini`、`opencode`、`openclaw`、`hermes`。

## 2. 签名

### 数据库

```sql
proxy_config(
  app_type TEXT PRIMARY KEY CHECK (app_type IN (
    'claude', 'claude-desktop', 'codex', 'gemini',
    'opencode', 'openclaw', 'hermes'
  )),
  enabled INTEGER NOT NULL DEFAULT 0,
  route_mode TEXT NOT NULL DEFAULT 'direct'
    CHECK (route_mode IN ('direct', 'proxy')),
  ...
)
```

- DB 列 `enabled` 是兼容存储名，领域含义只能是 `takeover_enabled`。
- schema v15 从旧三模块表迁移时必须整表重建；旧 `enabled=1` 映射为 `route_mode='proxy'`，`enabled=0` 映射为 `direct`。
- `Database::release_takeover_ownership(app_type)` 必须在同一事务中执行 `enabled=0` 与删除该模块 `proxy_live_backup`。
- `Database::list_proxy_route_takeovers()` 只返回 `enabled=1 AND route_mode='proxy'` 的模块。

### Rust / IPC

```rust
pub enum RouteMode { Direct, Proxy }

pub struct AppProxyConfig {
    pub app_type: String,
    #[serde(default, alias = "enabled")]
    pub takeover_enabled: bool,
    #[serde(default)]
    pub route_mode: RouteMode,
    // ...
}

pub struct ProxyModuleTakeoverStatus {
    pub takeover_enabled: bool,
    pub route_mode: RouteMode,
}

pub struct ProxyTakeoverStatus {
    pub claude: ProxyModuleTakeoverStatus,
    pub claude_desktop: ProxyModuleTakeoverStatus,
    pub codex: ProxyModuleTakeoverStatus,
    pub gemini: ProxyModuleTakeoverStatus,
    pub opencode: ProxyModuleTakeoverStatus,
    pub openclaw: ProxyModuleTakeoverStatus,
    pub hermes: ProxyModuleTakeoverStatus,
}

pub struct ProxyStopError {
    pub code: String,
    pub message: String,
    pub modules: Vec<String>,
}
```

`start_proxy_server` 只启动网关。`stop_proxy_server` 与兼容命令 `stop_proxy_with_restore` 都是受保护的用户停止：若仍有 proxy 路由模块，返回 `ProxyStopError { code: "proxyRoutesActive", modules: [...] }`。

模块接管与模式切换的稳定签名：

```rust
// IPC：旧调用省略 route_mode 时按 direct 处理。
set_proxy_takeover_for_app(
    app_type: String,
    enabled: bool,
    route_mode: Option<RouteMode>,
) -> Result<(), String>;

set_proxy_route_mode(
    app_type: String,
    route_mode: RouteMode,
) -> Result<(), String>;

// 服务层：调用方传规范 app_type；模式切换不重新 capture。
ProxyService::set_takeover_for_app(
    app_type: &str,
    enabled: bool,
    route_mode: RouteMode,
) -> Result<(), String>;

ProxyService::switch_route_mode(
    app_type: &str,
    route_mode: RouteMode,
) -> Result<(), String>;
```

所有 provider `add/save/update/switch/sync/reapply/remove` 路径必须先从 `proxy_config` 得到同一判定：

```rust
pub enum LiveWriteDecision {
    Skip,           // takeover_enabled=false
    DirectUpstream, // enabled + direct
    ProxyManaged,   // enabled + proxy
}
```

### TypeScript

```ts
type ProxyRouteMode = "direct" | "proxy";

interface ProxyModuleTakeoverStatus {
  takeoverEnabled: boolean;
  routeMode: ProxyRouteMode;
}

interface ProxyTakeoverStatus {
  claude: ProxyModuleTakeoverStatus;
  claudeDesktop: ProxyModuleTakeoverStatus;
  codex: ProxyModuleTakeoverStatus;
  gemini: ProxyModuleTakeoverStatus;
  opencode: ProxyModuleTakeoverStatus;
  openclaw: ProxyModuleTakeoverStatus;
  hermes: ProxyModuleTakeoverStatus;
}
```

`claude-desktop -> claudeDesktop` 必须通过单一映射表转换，禁止各消费方自行拼接 wire key。

## 3. 契约

### 三个正交维度

1. **网关运行态**：实际真相是 `ProxyService::get_status().running`；持久化 `proxy_enabled` 只是运行镜像，不是下次启动意图。
2. **接管所有权**：`takeover_enabled=false` 时 Agent-Switch 不得写该模块 live 配置。
3. **写入目标**：仅 `takeover_enabled=true` 时 `route_mode` 生效；`direct` 写真实上游，`proxy` 写本地网关目标。

单向依赖：启用/切换为 `proxy` 可以确保网关启动；启动网关永不自动接管，切为 `direct` 或关闭接管永不自动停止网关。

`takeover_enabled` 是七模块唯一 live 写权限，既约束接管命令，也约束 provider CRUD/switch/sync：

| 状态 | provider/current DB | 模块 live |
|---|---|---|
| `takeover_enabled=false` | 允许更新 | **禁止创建、改写或删除**（hands-off） |
| `enabled=true, route_mode=direct` | 允许更新 | 用模块原生 writer 写真实上游 |
| `enabled=true, route_mode=proxy` | 允许更新 | 只维护该模块本地命名空间与网关 token；不得写真实上游密钥 |

- provider 或 mode 热切换不得写 `proxy_live_backup.original_config`；该槽位只保存首次开启前快照。
- `manual disable` 对 direct/proxy 都精确恢复首次开启前状态；只有启动/异常恢复遇到历史 direct ownership 时才保留当前真实上游 live、仅放弃 ownership。
- provider/current/live 任一步失败必须补偿到操作前状态；报错后不得留下“provider 已变但 live 未变”“DB=off 但 live=AGS managed”或反向分裂。补偿失败时保留 immutable snapshot 与可恢复 ownership，并返回包含补偿错误的错误。

### 四模块独立命名空间

| 模块 | 规范入口 | RequestContext |
|---|---|---|
| Claude Desktop | `/claude-desktop/v1/messages`, `/claude-desktop/v1/models` | `AppType::ClaudeDesktop`, `claude-desktop` |
| OpenCode | `/opencode/v1/chat/completions`, `/opencode/v1/models` | `AppType::OpenCode`, `opencode` |
| OpenClaw | `/openclaw/v1/chat/completions`, `/openclaw/v1/responses`, `/openclaw/v1/messages`, `/openclaw/v1/models` | `AppType::OpenClaw`, `openclaw` |
| Hermes | `/hermes/v1/chat/completions`, `/hermes/v1/responses`, `/hermes/v1/messages`, `/hermes/v1/models` | `AppType::Hermes`, `hermes` |

- 每条入口必须先校验本地 gateway Bearer token，再按模块独立选择 provider、统计、故障转移和熔断；禁止复用内部硬绑 `AppType::Codex`/`Claude` 的 handler。
- adapter 按**每个候选 provider**的 canonical 协议选择；OpenAI Chat、OpenAI Responses、Anthropic Messages 不能跨协议混用 wire body。
- OpenCode `npm` 只允许能力矩阵精确 allowlist；OpenClaw `api`、Hermes `api_mode` 未知值，以及 Hermes `bedrock_converse`，仅在 proxy 路径原子拒绝。能力校验必须早于 snapshot capture、网关启动、live/provider/current/route DB 变更；direct 不受限制。
- takeover 关闭且无 snapshot/ownership 时，用户手工配置同名本地命名空间不属于 AGS 残留；crash recovery 不得清理或改写。

### 启动与退出

- 每次应用启动都把残留 `proxy_enabled=true` 对齐为 false，不据此启动网关。
- 启动不得按历史 `enabled` 调用 `set_takeover_for_app(..., true)`。
- 异常恢复必须发生在 `initialize_common_config_snippets` 前，确保 snippet 读取已恢复的 clean live。
- `proxy` 残留：仅在确有备份/占位时恢复；全目标成功后原子清所有权与快照；失败保留两者。
- `direct` 残留：不修改 live，只原子清所有权与快照。
- 应用退出使用内部清理路径；它与受保护的用户停止不是同一语义。

### 版本化精确快照

`proxy_live_backup.original_config` 保存版本化 manifest：

```json
{
  "version": 1,
  "app_type": "claude",
  "targets": [
    {"id": "settings", "kind": "file_bytes", "existed": true, "payload_base64": "..."},
    {"id": "auth", "kind": "semantic_json", "existed": false, "payload": null}
  ]
}
```

- `id` 是模块内稳定逻辑目标，不保存绝对用户路径。
- `file_bytes` 必须逐字节 round-trip，支持非 UTF-8；`existed=false` 表示恢复时删除 Agent-Switch 创建的目标。
- `semantic_json` 是保留给将来不能裸复制目标的通用能力；当前七模块精确快照均使用 `file_bytes`。
- 当前稳定目标集：Claude=`settings`；Codex=`auth/config/model_catalog`；Gemini=`.env`；Claude Desktop=`normal_config/threep_config/profile/meta`；OpenCode=`opencode.json`；OpenClaw=`openclaw.json`；Hermes=`config.yaml`。
- OpenCode 凭据在 `opencode.json`；接管、snapshot、restore **禁止读取或写入 `opencode.db`**（会话/用量库）。
- adapter 恢复前必须预检 `app_type`、完整且精确的 target id 集、kind 与 payload；未知/缺失 target 在零写入状态失败。
- 首次 capture 后不得因 provider 或 mode 切换覆盖快照。可变 `managed expected`（当前受管全文/指纹/代次）是 C3 的独立对象，不得复用 snapshot 槽。
- 多目标任一恢复失败时，不得删除快照或清 `takeover_enabled`；adapter 必须先预捕获全部当前目标，再逆序补偿已写目标并报告补偿失败。
- 无 `version` 的旧三模块 JSON 交给 legacy decoder；它只能 best-effort，不能标记为逐字节快照。

## 4. 校验与错误矩阵

| 条件 | 行为 |
|---|---|
| `route_mode` 不是 `direct`/`proxy` | DB CHECK 或 `RouteMode::from_str` 拒绝 |
| 快照版本不支持、app_type 非规范值、targets 为空或 id 重复 | manifest 校验失败；保留原快照与所有权 |
| 已存在有效快照时再次 capture | 返回已有快照，不覆盖 |
| proxy 多目标恢复任一失败 | 返回错误；保留 `enabled=1` 与快照 |
| 用户停止时存在 `takeover_enabled=true && route_mode=proxy` | 拒绝，`code=proxyRoutesActive`，返回准确模块列表 |
| 仅 direct 模块处于接管 | 允许停止网关，不修改模块 live |
| 启动时仅有旧 `enabled=1`、无备份且 live 已净化 | 不凭 enabled 重写 live；清残留所有权 |
| C2 adapter 尚未支持的新模块存在快照 | 报错并保留快照/所有权，禁止静默删除 |
| 四模块 proxy 协议不在能力矩阵（含 Hermes `bedrock_converse`） | 在 capture/live/DB 变更前拒绝；direct 仍可用 |
| provider/current 更新后 namespaced live 写失败 | byte-exact 回滚 live，并恢复 provider row 与 DB/settings current；补偿失败详情并入错误 |
| `takeover_enabled=false` 且无 snapshot，但 live 手工指向 C2b 命名空间 | 视为用户配置；启动恢复不得清理、改写或创建 ownership |
| manifest target 缺失、多余、kind/payload 错误 | 写任何目标前拒绝；live/ownership/snapshot 保持原样 |
| OpenCode 接管/恢复 | 只处理 `opencode.json`；`opencode.db` 内容与 mtime 保持不变 |

## 5. Good / Base / Bad Cases

- **Good**：Codex 接管为 `proxy`，用户停止网关得到含 `codex` 的结构化错误；关闭接管恢复首次快照后再停止成功。
- **Good**：OpenCode 关闭接管时切 provider 只改 DB；开启 direct 后写真实 `options.baseURL/apiKey`；切 proxy 后只写 `/opencode/v1` 与 gateway token，关闭逐字节恢复 `opencode.json`，`opencode.db` 始终不变。
- **Good**：四模块 provider 更新后 live writer 失败；返回错误前恢复 provider row、DB/settings current 和切换前 live bytes，immutable snapshot 不变。
- **Base**：仅 Claude 为 `direct` 且接管开启；网关可独立启动/停止，Claude live 不变。
- **Base**：用户手工把 Hermes 配置指向本地 `/hermes/v1`，但 takeover 关闭且无 snapshot；启动恢复保留该配置。
- **Bad**：应用启动读取 DB `proxy_enabled=true` 后自动启动网关，或读取旧 `enabled=1` 后重新写入占位配置。
- **Bad**：provider switch 在 `takeover_enabled=false` 时调用模块 writer；这会绕过唯一写权限并改写用户 live。
- **Bad**：OpenCode/OpenClaw/Hermes 复用 `/v1` Codex handler 或未知协议 fallback 到 CodexAdapter；会串用 provider/统计并发送错误 wire body。
- **Bad**：provider row/current 已提交后 namespaced live 写失败，只回滚 current 而不恢复 provider/live。
- **Bad**：在 v15 前用 `ALTER TABLE ADD route_mode`；这会保留旧三值 `app_type CHECK`，也会在迁移备份前污染旧库。

## 6. 必需测试

- v14 三行表迁移到 v15：七行齐全；旧 enabled 开启行变为 proxy，关闭行变为 direct；重复迁移幂等。
- 新库 create path 与旧库 `create_tables -> apply_schema_migrations` 全链路均成功，旧 CHECK 不被四个新 app 提前触发。
- DAO/wire：DB `enabled` 与 Rust `takeover_enabled` 显式映射；旧输入 `enabled` 可反序列化；输出只含 `takeoverEnabled`。
- failover/profile 只更新自己的字段，不把接管状态或 `route_mode` 重置为默认值。
- `ProxyTakeoverStatus` 序列化必须包含七个必填 key 与每项两个字段。
- 停止保护覆盖 proxy/direct 混合、纯 direct、无接管三种组合。
- 启动恢复覆盖 proxy 成功/失败、direct 放弃所有权、无备份旧状态、`proxy_enabled` 镜像归零，并断言不自动重接管。
- snapshot 覆盖非 UTF-8、目标原先不存在、多目标、完整 target 集预检、不覆盖已有快照、失败保留状态、legacy 分流。
- 七模块 provider 写权限矩阵：off 的 add/update/switch/sync 不写 live；direct 写真实上游；proxy 保留本模块 gateway endpoint/token 和 immutable snapshot；模块间互不修改。
- 四模块命名空间路由全部覆盖存在性与 Bearer 鉴权；请求断言 `AppType`、provider 选择、usage/统计不落到 Codex/Claude。能力矩阵内每种 canonical 协议覆盖 JSON、SSE、tool、error；Chat/Responses 候选不可跨协议。
- 能力矩阵外 proxy 拒绝断言发生在 capture/live/provider/current/route DB 前；同 provider 的 direct 成功。
- 四模块精确恢复覆盖 existed=false 删除、Claude Desktop 多文件补偿、OpenCode `opencode.json` round-trip 且 `opencode.db` 内容/mtime 不变、OpenClaw JSON5 与 Hermes YAML 逐字节恢复。
- provider/current/live 失败原子性：故意使 namespaced writer 失败，断言 provider row、DB/settings current、live bytes、immutable snapshot 都与操作前相同。
- 未拥有手工命名空间：`takeover_enabled=false`、无 snapshot 时 crash recovery 后逐字节不变。
- 测试必须使用临时 HOME 与全局锁，禁止读写真实用户配置。

## 7. Wrong vs Correct

### Wrong

```rust
// 把服务运行镜像当成启动意图，并把所有旧 enabled 解释为 direct。
if db.proxy_enabled().await? {
    proxy.start().await?;
}
let route_mode = RouteMode::Direct;

// provider CRUD 无视接管所有权，直接写用户 live。
db.save_provider(app.as_str(), &provider)?;
write_live_with_common_config(&db, &app, &provider)?;

// 四模块复用硬绑 Codex 的入口/adapter，未知协议也 fallback。
let adapter = CodexAdapter::new(provider.settings_config.clone());
router.route("/v1/chat/completions", post(handle_chat_completions));

// 只加列，旧 app_type CHECK 仍只允许三模块。
ALTER TABLE proxy_config ADD COLUMN route_mode TEXT DEFAULT 'direct';
```

### Correct

```rust
// 进程启动时服务实际未运行：先归零镜像，恢复残留但不重新接管。
db.set_proxy_enabled(false).await?;
proxy.recover_from_crash().await?;
initialize_common_config_snippets().await?;

// 每个 provider 写入点消费同一 SSOT；off 永远 hands-off。
match proxy.live_write_decision_for_app(&app).await? {
    LiveWriteDecision::Skip => save_db_only(&provider)?,
    LiveWriteDecision::DirectUpstream => write_real_upstream(&provider)?,
    LiveWriteDecision::ProxyManaged => write_module_namespace(&provider)?,
}

// 四模块用独立 app/context；proxy 能力门必须早于任何副作用。
validate_module_proxy_capability(&app, &provider)?;
capture_snapshot_once(&db, adapter).await?;
write_module_namespace(&provider)?;
commit_takeover_state().await?;
```

```sql
-- v15 在 savepoint 中整表重建，并保留历史语义。
CASE WHEN enabled = 1 THEN 'proxy' ELSE 'direct' END
```

## Scenario: C3 外部配置检测与冲突处理

### 1. Scope / Trigger

修改以下任一内容前必须遵守本节：

- 七模块 live 外部变化检测、防抖轮询或 worker 生命周期；
- managed expected / conflict / generation 状态；
- `external-config-changed` 事件或外部配置查询/接受/拒绝命令；
- 精确 route parser（`external_route_detection`）；
- AGS 自写 generation 抑制（ProxyService、ProviderService、MCP、Profile、import/S3/WebDAV/common config）。

C3 只交付后端检测、内存冲突、事件、状态查询与 accept/reject。UI 对话框与查询失效消费在 C4。

### 2. Signatures

```rust
// 事件
const EXTERNAL_CONFIG_CHANGED_EVENT: &str = "external-config-changed";

// wire camelCase
struct ExternalConfigChangedPayload {
    app_type: String,        // appType，规范值如 "claude-desktop"
    generation: u64,
    conflict: bool,
    takeover_enabled: bool,  // takeoverEnabled
}

struct ExternalConfigModuleStatus {
    app_type: String,
    generation: u64,
    conflict: bool,
    takeover_enabled: bool,
    route_mode: RouteMode,   // routeMode
}

// IPC
get_external_config_status()
    -> Result<Vec<ExternalConfigModuleStatus>, String>;
accept_external_config_change(app_type: String, generation: u64)
    -> Result<(), String>;
reject_external_config_change(app_type: String, generation: u64)
    -> Result<(), String>;

// 外层 orchestrator：调用方必须已持有 C2 per-app switch lock
struct ManagedWriteToken { app_type: AppType, generation: u64, ... }

ProxyService::begin_managed_write_locked(app)
    -> Result<Option<ManagedWriteToken>, String>;
ProxyService::finish_managed_write_locked(token)
    -> Result<(), String>;
ProxyService::abort_managed_write_locked(token)
    -> Result<(), String>;
// 同步 leaf（MCP/Provider）使用 *_locked_sync 变体

// 精确 route parser：只消费 capture 字节，不读 live 文件
parse_actual_route(app, capture, proxy_service, db)
    -> Result<RouteMode, String>;

// 导入后置同步：必须接真实 AppState
run_post_import_sync(app_state: &AppState) -> Result<(), AppError>;
```

- 事件与状态 **不** 携带文件全文、密钥、managed expected payload。
- 结构化错误 JSON 至少含 `code`；已落地：
  - `externalConfigStaleGeneration`
  - `externalConfigConflictNotFound`
  - `externalConfigTakeoverInactive`
- 默认轮询：poll `500ms`、防抖 `200ms`、full scan `5s`；mtime/len 只是候选。

### 3. Contracts

**两个对象必须分离：**

| 对象 | 存储 | 生命周期 | 可变性 |
|---|---|---|---|
| immutable restore snapshot | `proxy_live_backup.original_config` | 首次开启捕获；关闭时恢复 | 接管会话内不可变；C3 只读 |
| managed expected | 内存 | 接管开启时初始化；关闭/退出清 | 随 AGS 自写或 accept 更新；含全文+指纹+generation |

**检测：**

- 标准库 metadata 快扫 + 周期 full scan；禁止依赖 mtime/len 唯一真相。
- 同一模块连续两次相同 full capture 且过防抖窗口后才提交。
- 多 target 一次锁内采集；中间半写不得发事件。
- OpenCode 只监控 `opencode.json`，永不读/写 `opencode.db`。
- target id 复用 C2 adapter：Claude `settings`；Codex `auth/config/model_catalog`；Gemini `.env`；Claude Desktop `normal_config/threep_config/profile/meta`；OpenCode `opencode.json`；OpenClaw `openclaw.json`；Hermes `config.yaml`。

**判定：**

- `takeover_enabled=false`：只刷新 `last_observed`/generation，发 `conflict=false`；绝不写 live、provider/current/route DB。
- `takeover_enabled=true` 且 observed != expected：建/更新冲突，保留**首次** conflict.expected；后台永不自动接受或拒绝。
- observed == expected 或 managed write in-flight：抑制事件；observed 重新等于 expected **不会**自动清已有冲突。
- 冲突期间再次外变：刷新 observed 并推进 generation，首次 expected 不变。

**route parser：**

- 精确等于当前网关 origin/namespace => `Proxy`。
- 同网关 host 但 port/path/namespace 错、`localhost.evil` 等伪装主机 => 拒绝。
- 其它合法 HTTP(S)、可解析的非当前网关入口，**包括自托管私网/localhost** => `Direct`。
- 失败不得猜 Direct。OpenCode/OpenClaw/Hermes 只看当前 provider fragment；Claude Desktop 只看当前 profile/meta。

**accept：** 锁内二次校验 generation/conflict 后先 parse observed；成功后只更新 `route_mode`（保持接管开启）与内存 expected/last_observed/generation，清 conflict，发 `conflict=false`。不写 live/provider/snapshot。DB 失败则内存不提交。

**reject：** adapter 事务式恢复 conflict.expected；existed=false 删除；多目标失败补偿。成功后稳定 capture 必须逐字节等于 expected，再推进 generation 并清 conflict。失败保留 conflict/expected/generation/ownership/snapshot。

**自写抑制：** 所有 AGS live 写在已持有同一 `SwitchLockManager` per-app 锁的外层 orchestrator 上 `begin_managed_write_locked → write → finish/abort`。锁不可重入；leaf writer 禁止再取锁/token。成功 finish 在放锁前更新 expected 并清 conflict；失败先 C2 回滚再 abort，不得泄漏 in-flight。abort 时即使 capture 失败也必须清 in-flight；既有 conflict 可推进 generation，避免 UI 永久 stale。import/S3/WebDAV 后置同步必须复用真实 `AppState`，禁止 `AppState::new(db)` 隔离实例。

**生命周期：**

- setup：`reset_proxy_runtime_mirror` → `recover_from_crash` → common snippet 初始化 → `external_config_monitor.start()` 一次。
- 退出/显式 restart/更新安装：`cleanup_before_exit` 先 `stop` 并 await join，再 `stop_with_restore_keep_state`。
- `start` 幂等且“一生一次”；`stop` 后不隐式重启。
- 纯 `RESTART_EXIT_CODE` 默认路径不跑自定义清理；`restart_app` / `install_update_and_restart` 必须先显式 `cleanup_before_exit`。

### 4. Validation & Error Matrix

| 条件 | 行为 |
|---|---|
| accept/reject generation 过期 | `externalConfigStaleGeneration`；无副作用 |
| accept/reject 时当前无冲突 | `externalConfigConflictNotFound`；无副作用 |
| accept/reject 时接管已关 | `externalConfigTakeoverInactive`；无副作用 |
| accept 时 route 无法可靠解析 | 错误；conflict/expected/route_mode/live/snapshot 不变 |
| accept 时 DB 更新 route_mode 失败 | 错误；内存 conflict 保持 |
| reject 恢复任一 target 失败 | 错误；adapter 补偿；conflict/snapshot/ownership 保持 |
| reject 后 capture != expected | 错误；冲突保留 |
| managed write begin 重入 | 错误；不得覆盖活跃 generation |
| finish 的 generation 不匹配 | 错误；不得提交 expected；必要时 abort 清 in-flight |
| abort 且 capture 无效 | 仍必须清 in-flight；已有 conflict 保留并可推进 generation |
| worker 读失败/临时缺失 | 重试；不 emit、不写 DB、不清 snapshot |
| takeover off 外变 | 仅 refresh 事件；live/providers 表不变 |
| 生产 import 后同步构造隔离 AppState | 禁止 |

### 5. Good / Base / Bad Cases

- **Good**：接管 Claude 后外部改 `settings.json` → 冲突事件；accept 后 route_mode 同步、expected 更新、immutable snapshot 不变；reject 逐字节恢复后 poll 不立即复发。
- **Good**：OpenCode proxy 下 MCP 投影只改 `opencode.json` MCP 段，保留 `/opencode/v1` 与 gateway token；expected 更新且无外变事件。
- **Good**：SQL/S3/WebDAV 导入后用真实 AppState 同步；takeover off 不写 live。
- **Base**：未接管七模块外变只发 refresh；providers 表与 live 不被监听流程写入。
- **Base**：自托管 `http://10.0.0.5:8080/v1` 或 `http://localhost:9000/v1` 可被 accept 为 Direct。
- **Base**：冲突期间 observed 又改回 expected 时，冲突仍保留，等待用户 accept/reject 或后续 AGS 显式写成功清冲突。
- **Bad**：用 crash bool detector 或 prefix URL 判断 route。
- **Bad**：把 managed expected 写入 `proxy_live_backup` 或新建持久冲突表。
- **Bad**：leaf writer 与 orchestrator 双重 `lock_switch_for_app` / begin。
- **Bad**：import 后 `AppState::new(db)` 绕过运行中 monitor。

### 6. Tests Required

- 七模块修改/创建/删除：防抖后每模块一次事件；多文件只发稳定一次。
- 同 len/mtime 但内容变化：周期 full scan 仍能检出。
- takeover off：refresh 且 live/providers 不变。
- takeover on：冲突保留首次 expected；immutable snapshot 字节不变；observed==expected 不自动清冲突。
- route parser：七模块 exact Proxy/Direct；`localhost.evil`、旧端口、错 namespace、畸形/混合/缺 target/当前 provider 缺失拒绝；自托管私网/localhost Direct。
- accept/reject：成功路径、stale generation、conflict missing、takeover inactive、parse/DB/restore 失败无副作用；reject 后无立即复发。
- managed write：Proxy/Provider/MCP 成功抑制事件；失败 abort 清 in-flight、保留 conflict；无 generation 泄漏。
- worker start 幂等、stop join；crash recovery + 退出路径先停 monitor。
- import/S3/WebDAV 共用真实 AppState；OpenCode 不碰 `opencode.db`。
- TempHome + serial/global lock。

### 7. Wrong vs Correct

#### Wrong

```rust
// 把 managed expected 塞进 immutable snapshot 槽
db.save_live_backup(app, &managed_expected_json).await?;

// import 后断开运行时 monitor
let isolated = AppState::new(db.clone());
run_post_import_sync(&isolated)?;

// leaf 再取不可重入锁 / 重复 begin
let _outer = proxy.lock_switch_for_app(app).await;
write_leaf_that_also_locks_or_begins(app)?; // deadlock / re-entry
```

#### Correct

```rust
// crash recovery 与 common snippet 后启动一次；退出先 stop/join
proxy.recover_from_crash().await?;
initialize_common_config_snippets(&state);
monitor.start().await?;
// ...
// cleanup_before_exit 内：
monitor.stop().await?;
proxy.stop_with_restore_keep_state().await?;

// 外层锁 + 单 token 包完整写事务
let _guard = proxy.lock_switch_for_app(app.as_str()).await;
let token = proxy.begin_managed_write_locked(&app).await?;
match write_all_targets() {
    Ok(()) => proxy.finish_managed_write_locked(token).await?,
    Err(e) => {
        rollback();
        let _ = proxy.abort_managed_write_locked(token).await;
        return Err(e);
    }
}

// 生产后置同步复用真实 AppState
run_post_import_sync(&app_state)?;
```
