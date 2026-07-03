# 执行计划 — Claude Code 与 Codex 工具接管

> 配套 `prd.md` / `design.md`。按顺序实现,从简到难:DB → DAO → 服务层(检测→备份→写入→编排)→ API → 前端 → 质量门 → 运行验证。

## 实现顺序

### 1. 依赖与迁移

- [ ] `src-tauri/Cargo.toml` 增 `toml_edit = "0.22"`。
- [ ] `db/migrations.rs` 增 migration v4:`tool_takeover`、`tool_takeover_backups`(见 design §3)。不动 v1-v3。

### 2. DAO 层 `db/dao/tool_takeover.rs`

- [ ] `get_state(tool)` / `upsert_state(tool, enabled, last_applied_at, last_target, last_error)`。
- [ ] `set_enabled(tool, enabled)`(关闭路径只改 enabled,不动其它)。
- [ ] `insert_backup(record)` / `list_backups(tool)`。
- [ ] `db/dao/mod.rs` 注册模块。

### 3. 服务层 `services/tool_takeover/`

- [ ] `mod.rs`:常量(LOCAL_BASE / 路径段 / PLACEHOLDER_TOKEN / CODEX_PROVIDER_ID)、`Tool` 枚举、路径解析(`dirs::home_dir()`)、原子写 helper(`.tmp` + rename + `create_dir_all`)。
- [ ] `claude_code.rs`:`detect()`(读 settings.json → 四态分类)、`apply()`(serde_json 合并写 env 两键)。
- [ ] `codex.rs`:`detect()`(toml_edit 读 model_provider + base_url)、`apply()`(toml_edit 写 provider 表 + 顶层 model_provider;serde_json 合并写 auth.json 的 OPENAI_API_KEY)。
- [ ] `backup.rs`:`backup_before_write(tool, path, target)` —— R3.4 已接管态跳过复制;原文件存在则复制+记录,不存在则记录 original_existed=0。
- [ ] `mod.rs` 编排:`enable(tool)`(备份→apply→upsert_state 成功/失败)、`disable(tool)`(仅 set_enabled=0)、`reapply(tool)`(要求 enabled,再 apply)、`status(tool)`(detect + state)、`list_backups(tool)`。
- [ ] 失败路径:apply 出错 → 写 last_error、返回 Err,不 panic。
- [ ] `services/mod.rs` 注册 `pub mod tool_takeover;`。

### 4. HTTP API `http/api/tools.rs`

- [ ] 路由:`GET /`、`GET /{tool}`、`POST /{tool}/takeover`、`POST /{tool}/reapply`、`GET /{tool}/backups`(见 design §7)。
- [ ] `tool` 校验:claude-code / codex / opencode;opencode 的 takeover/reapply → 400 not_supported。
- [ ] `http/api/mod.rs` 导出;`http/router.rs` 在 `/api/{*path}` 之前 `.nest("/api/tools", api::tools::routes())`。

### 5. 前端

- [ ] `lib/api.ts`:`toolsApi`(list/get/setTakeover/reapply/backups)+ 类型 `ToolStatus`/`ToolBackup`;queryKey `['tools']`。
- [ ] `components/tools/ToolCard.tsx`:开关 + 四态指向徽标 + 最近写入时间 + 错误 + 备份列表 + 复制恢复说明 + 风险提示 + 开启前确认。
- [ ] `components/tools/OpenCodeCard.tsx`:手动说明 + 可复制片段,无开关。
- [ ] 重写 `pages/ToolsPage.tsx`:三张卡片组合,移除 PagePlaceholder。

### 6. 质量门(AC12)

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo fmt && cargo fmt --check
cargo check                                   # 0 warning
cargo clippy --all-targets -- -D warnings
cd .. && npm run build
```

### 7. 运行验证(启动 .exe + curl 127.0.0.1:42567)

- [ ] 迁移 v4 成功,/health ok。
- [ ] `GET /api/tools` 返回三工具,opencode supports_takeover=false。
- [ ] 在隔离的临时 HOME 下验证(避免污染真实 ~/.claude、~/.codex):
  - 开启 claude-code 接管 → settings.json env 两键正确、占位符、原有键保留;备份记录生成。
  - 开启 codex 接管 → config.toml model_provider/provider.base_url 正确、auth.json OPENAI_API_KEY 占位符、其它配置保留;备份记录生成。
  - 文件不存在场景 → 创建并写成功,备份记录 original_existed=0。
  - 再次开启(已接管态)→ 不产生覆盖好备份(R3.4)。
  - 关闭接管 → 文件不变,enabled=0。
  - 检测四态:agent-switch / 官方 / 第三方(手造)/ 未配置。
  - opencode takeover → 400。
- [ ] 验证后清理测试数据,不残留写入真实工具配置。

## 风险与回滚点

- **风险:污染真实工具配置**。验证必须用临时 HOME(Windows 设 `USERPROFILE`)或备份真实 `~/.claude`、`~/.codex`。
- **风险:toml_edit 写坏 config.toml**。重点测合并保留(注释/其它表/其它 provider)。
- **回滚**:本任务全是新增模块 + 新迁移 + ToolsPage 重写;回滚即 `git checkout` 相关文件(迁移 v4 已应用的 DB 在开发期可删库重建)。

## 验证命令速查

```bash
# 隔离 HOME 启动(Windows bash)
USERPROFILE="/tmp/as-test-home" ./src-tauri/target/release/agent-switch.exe &
curl -s 127.0.0.1:42567/api/tools
curl -s -X POST 127.0.0.1:42567/api/tools/claude-code/takeover -H 'Content-Type: application/json' -d '{"enabled":true}'
cat /tmp/as-test-home/.claude/settings.json
```

## 实现方式

inline(主代理直接实现,参考上一子任务 model-management-refresh-alias 的流程);Phase 2 经 `trellis-before-dev` 加载 spec。如需并行可拆 `trellis-implement`,但本任务体量适中,inline 即可。
