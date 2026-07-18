# 本地网关与模块接管

## 1. 适用范围 / 触发条件

修改以下任一内容前必须遵守本文：

- `proxy_config` schema、迁移或 DAO；
- 本地网关启动/停止命令；
- 七模块接管状态、`route_mode` 或前后端 wire 类型；
- live 配置接管、异常恢复、退出清理或精确快照；
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
- `semantic_json` 用于 SQLite 等不能裸复制的目标，由模块 adapter 用事务 API 恢复。
- 首次 capture 后不得因 provider 或 mode 切换覆盖快照。
- 多目标任一恢复失败时，不得删除快照或清 `takeover_enabled`；adapter 负责补偿已写目标。
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

## 5. Good / Base / Bad Cases

- **Good**：Codex 接管为 `proxy`，用户停止网关得到含 `codex` 的结构化错误；关闭接管恢复首次快照后再停止成功。
- **Base**：仅 Claude 为 `direct` 且接管开启；网关可独立启动/停止，Claude live 不变。
- **Bad**：应用启动读取 DB `proxy_enabled=true` 后自动启动网关，或读取旧 `enabled=1` 后重新写入占位配置。
- **Bad**：在 v15 前用 `ALTER TABLE ADD route_mode`；这会保留旧三值 `app_type CHECK`，也会在迁移备份前污染旧库。

## 6. 必需测试

- v14 三行表迁移到 v15：七行齐全；旧 enabled 开启行变为 proxy，关闭行变为 direct；重复迁移幂等。
- 新库 create path 与旧库 `create_tables -> apply_schema_migrations` 全链路均成功，旧 CHECK 不被四个新 app 提前触发。
- DAO/wire：DB `enabled` 与 Rust `takeover_enabled` 显式映射；旧输入 `enabled` 可反序列化；输出只含 `takeoverEnabled`。
- failover/profile 只更新自己的字段，不把接管状态或 `route_mode` 重置为默认值。
- `ProxyTakeoverStatus` 序列化必须包含七个必填 key 与每项两个字段。
- 停止保护覆盖 proxy/direct 混合、纯 direct、无接管三种组合。
- 启动恢复覆盖 proxy 成功/失败、direct 放弃所有权、无备份旧状态、`proxy_enabled` 镜像归零，并断言不自动重接管。
- snapshot 覆盖非 UTF-8、目标原先不存在、多目标、semantic target、不覆盖已有快照、失败保留状态、legacy 分流。
- 测试必须使用临时 HOME 与全局锁，禁止读写真实用户配置。

## 7. Wrong vs Correct

### Wrong

```rust
// 把服务运行镜像当成启动意图，并把所有旧 enabled 解释为 direct。
if db.proxy_enabled().await? {
    proxy.start().await?;
}
let route_mode = RouteMode::Direct;

// 只加列，旧 app_type CHECK 仍只允许三模块。
ALTER TABLE proxy_config ADD COLUMN route_mode TEXT DEFAULT 'direct';
```

### Correct

```rust
// 进程启动时服务实际未运行：先归零镜像，恢复残留但不重新接管。
db.set_proxy_enabled(false).await?;
proxy.recover_from_crash().await?;
initialize_common_config_snippets().await?;
```

```sql
-- v15 在 savepoint 中整表重建，并保留历史语义。
CASE WHEN enabled = 1 THEN 'proxy' ELSE 'direct' END
```
