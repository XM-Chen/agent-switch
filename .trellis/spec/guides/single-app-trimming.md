# 单应用跨层裁剪指南

## 原则

删除一个客户端不是隐藏 tab。ccs 的应用身份贯穿前端、Rust、SQLite、live 文件、代理、MCP/Skills、Sessions、Deep Link、tray、测试和文档。每个批次先画引用闭包，再按依赖顺序删除。

目标只保留 Claude Code 客户端，但 **Claude Provider 的上游类型不等于客户端**：Codex OAuth、GitHub Copilot、OpenRouter、OpenAI Responses 适配需要保留。

## 强制检查清单

### 1. 后端 AppType

- `src-tauri/src/app_config.rs`：enum、`as_str`、`FromStr`、`all`、错误文案；
- switch vs additive：Claude/Codex/Gemini 是 switch；OpenCode/OpenClaw/Hermes 是 additive（`app_config.rs:369-392`）；
- `MultiAppConfig::default`、CommonConfigSnippets、visible/current/path override；
- command 参数解析和所有 `AppType::all()` 遍历。

### 2. 前端 AppId

- `src/lib/api/types`、`src/config/appConfig.tsx:17-110`、`src/App.tsx:124-216`；
- `AppSwitcher`、可见应用设置、默认/回退、localStorage；
- 图标、预设、capability 过滤、query key、event payload；
- MCP/Skills app lists。

### 3. Provider/live

- `read_live_settings`、`write_live_snapshot`、import/default/seed/backfill；
- app-specific config modules；
- Provider CRUD、切换、删除、tray、terminal；
- proxy takeover 与普通模式；
- 完整 `settings_config` 未知字段往返测试。

### 4. 外围模块

- MCP 只投影 Claude；
- Prompts 只同步 CLAUDE.md；
- Skills 只投影 Claude Code skills；
- Sessions 只扫描/恢复 Claude Code；
- Deep Link app 固定 Claude，资源 provider/prompt/mcp/skill 保留；
- Usage 只留 Claude 请求/会话来源；
- OpenClaw workspace view/command 删除；Agents 占位页删除。

### 5. Proxy 与上游保护

**保护白名单：**

- `ProviderType::CodexOAuth` / `GitHubCopilot` / `OpenRouter`；
- `commands/codex_oauth.rs` / `commands/copilot.rs`；
- OAuth/account/quota/model/auth/refresh；
- Claude Provider 表单接线；
- `streaming_responses.rs` / `transform_responses.rs`；
- Claude 实际引用的 Codex Chat/Copilot transform/streaming 闭包。

删除独立 Codex 客户端升级/live/session UI，不按文件名批量删除 `codex*`。

### 6. DB 与同步

- schema v11 暂留，不 drop 多应用列；
- 停止创建/读取非 Claude 行；
- backup/sync artifact 只包含本产品业务数据；
- seed/import-before-seed 顺序；
- remote root 与产品身份隔离。

### 7. 注册与残留

- 前端 exports/routes/imports；
- Rust `mod`/`pub use`/`generate_handler!`/state/plugin；
- Tauri capabilities/config；
- i18n key、fixtures、tests、docs、CI/release assets；
- 精确残留搜索并维护允许列表（OAuth/Copilot/Responses/来源归属）。

## 推荐删除顺序

1. UI 导航和页面（保持编译）；
2. 前端 hooks/API/types/config；
3. Tauri commands 和 handler 注册；
4. services/live adapters/proxy 独立客户端分支；
5. 外围投影与导入；
6. module/export/dependency/tests；
7. 仅在引用归零后删文件；
8. 每批完整质量门 + Windows no-bundle build + 残留扫描。

## 禁止事项

- 只隐藏入口；
- 为“数据库干净”在首期破坏 schema；
- 把 provider type 当 AppType；
- 从旧 Agent Switch 0.2.2 spec 直接复制实现；
- 在一个大提交同时裁客户端、改品牌、改 updater；
- 用 allow/dead_code 大面积压住删除不完整。
