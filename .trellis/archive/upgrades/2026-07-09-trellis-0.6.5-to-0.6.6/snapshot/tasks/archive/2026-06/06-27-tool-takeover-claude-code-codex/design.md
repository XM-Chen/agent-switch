# 设计 — Claude Code 与 Codex 工具接管

> 配套 `prd.md`。本文件只写技术设计:边界、数据模型、配置写入契约、检测算法、API、前端、依赖与取舍。

## 1. 架构与分层

复用现有应用层分层(见 `spec/guides/app-stack-conventions.md`):

```text
HTTP handler (http/api/tools.rs)   仅参数校验 + 响应组装
        │
服务层 (services/tool_takeover/)   接管编排、备份、写入、检测
        │
DAO 层 (db/dao/tool_takeover.rs)   仅 SQL,读写接管状态与备份记录表
        │
SQLite (migration v4)
```

- handler 不直接读写工具配置文件,不拼 SQL。
- 文件读写、TOML/JSON 合并、备份全部在服务层。
- 接管状态/备份记录的持久化经 DAO 层。
- 工具配置文件路径解析、占位符常量集中在服务层一处定义。

### 服务层模块拆分

```text
src-tauri/src/services/tool_takeover/
├── mod.rs          编排:enable / disable / reapply / status / list_backups;占位符常量、工具枚举
├── backup.rs       写前备份:复制原文件到备份目录 + 写备份记录;R3.4 防覆盖判断
├── claude_code.rs  Claude Code:settings.json 读 / 合并写 / 检测
└── codex.rs        Codex:config.toml(toml_edit)+ auth.json(serde_json)读 / 合并写 / 检测
```

> 现有 `services/` 为扁平文件;本任务因涉及多文件协作(编排 + 备份 + 两个工具写入器),采用子模块目录,符合「在现有分层内增加模块」的原则。

## 2. 常量与路径契约

集中定义于 `services/tool_takeover/mod.rs`:

```rust
pub const LOCAL_BASE: &str = "http://127.0.0.1:42567";
pub const CLAUDE_CODE_PATH: &str = "/claude-code";   // 完整: LOCAL_BASE + 该段
pub const CODEX_PATH: &str = "/codex";
pub const PLACEHOLDER_TOKEN: &str = "agent-switch-managed"; // R2.3 占位符,绝不写真实密钥
pub const CODEX_PROVIDER_ID: &str = "agent-switch";          // model_provider 值 + provider 表名
```

工具配置路径(`dirs::home_dir()` 解析,已有 `dirs` 依赖):

| 工具 | 文件 | 字段 |
|------|------|------|
| Claude Code | `~/.claude/settings.json` | `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN` |
| Codex | `~/.codex/config.toml` | 顶层 `model_provider`、`[model_providers.agent-switch]` |
| Codex | `~/.codex/auth.json` | `OPENAI_API_KEY` |

备份目录:`<app_data_dir>/backups/tools/`(`config::paths::app_data_dir()`);备份文件名 `<tool>-<原文件名>-<时间戳>.bak`。

## 3. 数据模型(migration v4)

新增迁移 v4,**不改动已部署的 v1-v3**:

```sql
-- 每工具接管状态(单行一工具)
CREATE TABLE IF NOT EXISTS tool_takeover (
    tool             TEXT PRIMARY KEY,        -- 'claude-code' | 'codex'
    enabled          INTEGER NOT NULL DEFAULT 0,
    last_applied_at  TEXT,                     -- 最近成功写入时间
    last_target      TEXT,                     -- 最近写入的 base URL
    last_error       TEXT,                     -- 最近写入/操作错误(成功时清空)
    updated_at       TEXT NOT NULL
);

-- 接管写入前的备份记录(可多条,保留历史)
CREATE TABLE IF NOT EXISTS tool_takeover_backups (
    id               TEXT PRIMARY KEY,
    tool             TEXT NOT NULL,
    original_path    TEXT NOT NULL,
    backup_path      TEXT NOT NULL,            -- 原文件不存在时为空串,original_existed=0
    original_existed INTEGER NOT NULL DEFAULT 1,
    takeover_target  TEXT,                     -- 本次写入指向的 base URL
    created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tool_backups ON tool_takeover_backups(tool, created_at);
```

- 接管状态用专表而非 `app_metadata`:每工具多字段(enabled/last_applied/last_target/last_error),专表更清晰且便于扩展 OpenCode 之外的工具。
- `original_existed=0` 对应 R2.6:原文件缺失时记录「接管前为空」,作为可识别的备份标记。

## 4. 配置写入契约(核心)

### 4.1 Claude Code(`settings.json`,JSON 合并)

读取原文件(无则空对象)→ 用 `serde_json::Value` 合并 → 原子写回(临时文件 + rename)。

写入后等价于:

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567/claude-code",
    "ANTHROPIC_AUTH_TOKEN": "agent-switch-managed"
  }
}
```

合并规则:保留顶层其它键;保留 `env` 内其它键,只覆盖上述两个键。

### 4.2 Codex(`config.toml`,toml_edit 外科式编辑)

用 `toml_edit::DocumentMut` 解析原文件(无则空文档),保留用户注释与其它表,只设置:

```toml
model_provider = "agent-switch"

[model_providers.agent-switch]
name = "agent-switch"
base_url = "http://127.0.0.1:42567/codex"
wire_api = "responses"
requires_openai_auth = true
```

- `wire_api = "responses"`:与 `ccs` 一致(Codex 走 responses 协议)。
- 保留用户原有的 `model`、`model_reasoning_effort`、其它 `[model_providers.*]` 等。
- 顶层 `model_provider` 被改写为 `agent-switch`(接管即切换到我方 provider)。

### 4.3 Codex(`auth.json`,JSON 合并)

```json
{ "OPENAI_API_KEY": "agent-switch-managed" }
```

合并规则:只覆盖 `OPENAI_API_KEY`;保留 `tokens`、`last_refresh` 等官方登录字段(AC3)。

### 4.4 原子写

所有写入:写到同目录 `.<name>.tmp` 再 `fs::rename` 覆盖,避免写一半损坏用户配置。父目录不存在时先 `create_dir_all`。

## 5. 备份与防覆盖(R3)

`backup.rs::backup_before_write(tool, original_path, target)`:

1. 检测当前文件是否已是接管态(见 §6 检测):若**已是 agent-switch 态**,跳过文件复制(不产生新备份),避免用接管配置覆盖好备份(R3.4);仍可更新 `last_applied_at`。
2. 否则:原文件存在 → 复制到备份目录,写 `tool_takeover_backups`(`original_existed=1`);原文件不存在 → 仅写记录(`original_existed=0`,`backup_path=''`)。
3. 备份永不自动删除(安全优先;清理留待后续)。

## 6. 当前指向检测(R5,只读)

`detect(tool) -> ToolLiveStatus`,枚举类别:

```rust
enum TakeoverTarget { AgentSwitch, Official, ThirdParty, Unconfigured, Unrecognized }
```

- Claude Code:读 `settings.json` → `env.ANTHROPIC_BASE_URL`。
  - 缺文件/缺字段 → `Unconfigured`
  - `== LOCAL_BASE + /claude-code` → `AgentSwitch`
  - 含 `anthropic.com` 或空 → `Official`
  - 其它非空 → `ThirdParty`
  - JSON 解析失败 → `Unrecognized`
- Codex:读 `config.toml` → `model_provider` + 对应 `[model_providers.*].base_url`。
  - `model_provider == "agent-switch"` 且 base_url 命中我方 → `AgentSwitch`
  - 缺文件/缺 provider → `Unconfigured`
  - 指向 `api.openai.com`/默认 → `Official`
  - 其它 → `ThirdParty`;TOML 解析失败 → `Unrecognized`

检测不抛错到顶层:解析失败映射为 `Unrecognized` + 内部日志。

## 7. HTTP API(`http/api/tools.rs`)

挂载(`http/router.rs`,在 `/api/{*path}` catch-all 之前):

```rust
.nest("/api/tools", api::tools::routes())
```

契约:

```text
GET    /api/tools
       → [{ tool, supports_takeover, enabled, live_target, last_applied_at, last_error }]
       claude-code/codex: supports_takeover=true;opencode: false

GET    /api/tools/{tool}
       → { tool, supports_takeover, enabled, live_target, last_applied_at, last_error, backups:[...] }

POST   /api/tools/{tool}/takeover    body { enabled: bool }
       enabled=true  → 备份 + 写入 + 置 enabled=1(失败写 last_error 并返回 4xx/5xx)
       enabled=false → 仅置 enabled=0,停止写入,不改工具文件,返回 204/200
       tool=opencode → 400 not_supported

POST   /api/tools/{tool}/reapply     无 body
       幂等重新应用接管(R4.2);要求 enabled=1,否则 409

GET    /api/tools/{tool}/backups
       → [{ id, original_path, backup_path, original_existed, takeover_target, created_at }]
```

- handler 只校验 `tool ∈ {claude-code, codex, opencode}` 与 body,调用服务层。
- 响应不含任何密钥(本就只写占位符,且备份文件路径不含内容)。

## 8. 前端(`pages/ToolsPage.tsx` + `components/tools/` + `lib/api.ts`)

- `lib/api.ts` 增 `toolsApi`:`list()` / `get(tool)` / `setTakeover(tool, enabled)` / `reapply(tool)` / `backups(tool)`;queryKey `['tools']`。
- `ToolsPage`:三张卡片。
  - `components/tools/ToolCard.tsx`:Claude Code / Codex 通用——接管开关(Switch)、当前指向徽标(四态配色)、最近写入时间、最近错误、备份位置列表 + 「复制恢复说明」按钮、风险提示文案。
  - `components/tools/OpenCodeCard.tsx`:手动配置说明 + 可复制片段(指向 `/v1` 或后续兼容入口),无开关。
- 恢复说明文案(可复制):列出备份文件路径与「手动用备份覆盖原文件即可还原」的中文步骤。**不提供写回按钮**。
- 开关交互:开启前弹确认(风险提示);切换调用 `setTakeover`,成功后 invalidate `['tools']`。

## 9. 依赖

- 新增 `toml_edit = "0.22"`(Codex `config.toml` 外科式编辑,保留注释/其它表)。
- 复用:`serde_json`(JSON 合并)、`dirs`(home 目录)、`uuid`(备份记录 id)、`time`(时间戳)。

## 10. 关键取舍

1. **占位符令牌而非真实密钥**(R2.3/AC10):本地服务不做鉴权(父 PRD 安全边界),工具仍要求非空 token,故写固定占位符;真实凭据由路由层(子任务 5)在转发时注入上游。与 `ccs` 的 `PROXY_TOKEN_PLACEHOLDER` 思路一致。
2. **接管内容基本静态**:我方 base URL 固定 + 占位符固定,故 `reapply` 主要用于自愈(配置被外部改动后重写)与未来扩展;MVP 下开启即写一次已足够,`reapply` 作为幂等入口保留。
3. **不做一键恢复**(用户确认):仅展示备份位置 + 恢复说明,避免本应用主动写回用户配置带来的语义复杂度与风险。
4. **接管 ≠ 转发**(范围确认):本任务后工具已指向 agent-switch,但 `/claude-code`、`/codex` 仍是 501 占位,端到端跑通由子任务 5 交付。验收只验「配置写对 + 备份 + 检测」。
5. **Codex 切换 provider**:接管把顶层 `model_provider` 改为 `agent-switch`,符合「指向本地服务」语义;关闭接管不还原,用户原 provider 表仍在 `config.toml` 中(被保留),可手动改回。

## 11. 与其它子任务的衔接

- 子任务 5 `routing-failover-core`:实现 `/claude-code/*`、`/codex/*` 真实转发后,接管才端到端生效。接管写入的 base URL 段(`/claude-code`、`/codex`)即子任务 5 的入口契约。
- 子任务 8 `import-export-settings`:导入后接管开关须统一关闭/「曾开启需重新确认」,不得自动写工具配置(父 PRD 已定);本任务的 `tool_takeover` 表是其导入对象之一。
