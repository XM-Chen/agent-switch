# HTTP / Proxy 规范

## 本地服务边界

- 默认绑定 `127.0.0.1:42567`。
- `/api/*` 为管理 API；`/claude-code/*`、`/codex/*`、`/v1/*` 为代理入口；`/health` 为健康检查。
- 第一版本地入口无认证，必须避免日志和错误响应泄漏敏感凭据。

## 故障转移

- 已向客户端输出 stream chunk 后禁止 fallback。
- 未输出前遇到 retryable 错误，优先同端点重试；同端点重试间隔为 `SAME_ACCOUNT_RETRY_DELAY_MS`。
- `AuthError` 冷却为 300s，避免失效凭据端点被短周期反复选中。
- `cooldown_multiplier` 仅作用于通用上游/网络类冷却，不覆盖 AuthError 或上游 Retry-After 固定语义。

## SSE / stream guard

- stream 首块为空必须返回明确 retryable error，不得返回空流让客户端挂起。
- inline SSE error 在未输出前应转换为 `retryable=true, stream_started=false` 的 `ProxyError`。
- 上游 stream error 只发一次协议化错误事件，并设置终止状态，避免重复错误帧或 flush 残留行。

## OAuth refresh

- 预检刷新必须设置连接/总超时。
- refresh 响应缺少新 refresh_token 时保留旧 refresh_token，并记录可诊断日志。

## 响应头

只透传安全诊断头，如 `x-request-id`、rate-limit 系列；继续过滤 hop-by-hop 与敏感头。

## 管理 API 路由注册顺序

- `nest` 的具体前缀（如 `/api/providers`）必须注册在 `/api/{*path}` 兜底之前，否则被兜底吞掉。
- 同一 `Router` 内固定段与参数段冲突时（`/reorder` vs `/{id}`），固定段优先由 axum 静态段优先级保证，但仍应把固定段路由写在参数段之前以免误读。

## Provider 切换的原子性（set_current + 接管）

切换要同时改两处状态：DB 的 `is_current`（`providers` 表 partial unique index 保证互斥）与工具接管配置（`tool_takeover`）。二者必须一致，禁止"DB 说 current 是 A 但工具没接管"。

契约（`POST /api/providers/{id}/switch`）：

1. 查目标 provider，不存在 → 404。
2. 解析 `app_type` → `Tool`，`supports_takeover()` 为假（如 opencode）→ 400。
3. 记录切换前的 current **完整行**（不只是 id）：既用于失败回滚，也作为 `prev` 传给接管闭包供 Claude Code backfill 用。若 prev 就是目标自身（重复切换同一个）则不作为独立 `prev` 传入。
4. **先** `set_current`（DB），**再**按 `Tool` 分流接管：
   - **Claude Code**：走 `switch_claude(db, data_dir, prev, target, crypto)` 快照编排（回填保护 + Common Config 三层，见数据库规范「Claude Code 快照层」）。内部顺序：解析连接层（direct 先解密，失败早退无副作用）→ backfill `prev`（或首切时 backfill target 自身）→ `backup_before_write` → 写快照层整文件覆盖 → `apply`/`apply_direct` 注入连接层 → `upsert_state`。
   - **Codex 等**：沿用「仅连接层」接管（`proxy`→`enable`，`direct`→`enable_direct`），逐字节保持现状，不引入快照模型。
5. **接管失败必须回滚 `is_current`**：恢复到切换前的 current，若原本无 current 则 `clear_current`。回滚本身再失败时，把两个错误一并返回 500。
6. `direct` 模式 crypto 不可用 → 503，**不静默降级**为 proxy。

因为接管服务把 `tool_takeover` 状态的 `upsert_state` 放在最后一步，接管失败时该状态保持不变，与回滚后的 `is_current` 自然一致。Claude Code 快照编排在写任何文件前先解析（并解密）连接层，解析失败即早退，同样不留副作用。

删除 current provider：先 `clear_current`，若 `tool_takeover.active_provider_id` 仍指向被删 id，用 `set_mode` 复位为 `proxy`/`None`，避免悬空 direct 引用。

## MCP 管理（`~/.claude.json`，独立于切换）

Claude Code 的 MCP 服务器配置在 **`~/.claude.json`**——这是 `~/.claude/` 目录的**兄弟文件**，与切换接管的 `~/.claude/settings.json` 是两个不同文件。MCP 管理（`services/mcp/` + `http/api/mcp.rs` + `mcp_servers` 表）是**独立全局清单**，与 `tool_takeover` 完全解耦：**不**挂在 `perform_switch` 上，CRUD 后即时同步。

- **全量投影**：`mcpServers` 字段整体由 DB 里 `enabled_claude=1` 的集合决定（`sync_enabled_to_claude`）。读整个 `~/.claude.json` → 只替换 `mcpServers` 键 → 原子写，保留其它顶层键（`hasCompletedOnboarding` 等用户数据）。用户手加进 live 但不在 DB 的 server 会被抹掉，靠显式「从 live 导入」（`import_from_claude`）缓解。
- **未安装跳过**：`~/.claude.json` 与其兄弟 `~/.claude/` 目录均不存在时，同步为 no-op，不凭空建文件。判断兄弟目录用 **path 自身的 parent** 推导（`path.parent().join(".claude")`），不查真实 home——生产语义正确（两者同在 home 下），测试用临时目录时天然隔离。
- **Windows `cmd /c` 包装**：stdio 类型且 command ∈ {npx,npm,yarn,pnpm,node,bun,deno} 时改写为 `cmd /c <command> <args>`（`#[cfg(windows)]`）；目标路径为 WSL（`\\wsl$\` / `\\wsl.localhost\`）时跳过。
- **不加密**：MCP 规范本就明文写进 `~/.claude.json` 供 Claude Code 读，DB 存明文与 live 一致，不走 crypto。这与 endpoints/accounts 的 direct 凭证加密边界互不重叠。
