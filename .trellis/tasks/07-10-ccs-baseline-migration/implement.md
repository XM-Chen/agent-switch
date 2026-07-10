# Implement：ccs v3.16.5 基线与定制分支执行计划

> 本文件是父任务执行路线。产品实现拆给子任务；未经用户审核本计划并批准启动，不执行分支切换、删除或产品代码修改。

## 0. 执行前门禁

- [ ] 用户审核 `prd.md` / `design.md` / 本计划。
- [ ] 再次确认当前任务路径与状态仍为 `planning`。
- [ ] 再次确认 `main` HEAD、`origin/main`、工作树未跟踪/已修改内容；若与规划证据变化，先停下更新 PRD。
- [ ] 再次确认 `ref-cc-switch` 官方 tag peeled commit：`v3.16.5^{}` = `8d1b3306d09a27b9d8fc29694791d8421aba5f93`。
- [ ] 未获得单独授权时：不 push、不改默认分支、不发布、不删除外部备份。

## 1. 创建父/子任务结构（仍属规划）

- [ ] 保持本任务为父任务，补入子任务图与跨子任务验收。
- [ ] 创建并 link 子任务（建议 slug）：
  1. `ccs-baseline-bootstrap`
  2. `ccs-claude-only-trim`
  3. `ccs-windows-zh-trim`
  4. `ccs-agent-switch-identity`
  5. `ccs-updater-release`
  6. `ccs-retained-features-regression`
- [ ] 每个子任务分别完成 PRD；复杂子任务补 design/implement；按依赖逐个 start，不直接 start 父任务做所有代码。

## 2. Bootstrap 子任务：保护当前工作树

### 2.1 只读盘点

- [ ] 记录：

```bash
git status --short --branch
git rev-parse main
git rev-parse origin/main
git rev-parse 'v3.16.5^{}'
git branch --list agent-switch-ccs
```

- [ ] 若 `agent-switch-ccs` 已存在，停止并调查，不覆盖。
- [ ] 若 tracked 文件有未提交改动，停止并让用户决定（不可自行 stash/commit）。

### 2.2 外部 bootstrap 备份

- [ ] 在仓库外创建任务专属备份目录（建议 `E:/SynologyDrive/git_files/agent-switch-bootstrap-07-10/`；实际路径执行时记录）。
- [ ] 复制并校验：
  - `.trellis-upgrade-audit.json`
  - `.trellis/archive/`
  - `.trellis/tasks/07-10-ccs-baseline-migration/`
- [ ] 写 `manifest.txt/json`：源路径、大小、文件数、SHA-256（大目录可对文件清单做哈希）。
- [ ] 验证备份可读后才继续。此目录不自动删除，直到新分支 Trellis 迁入并验证完成且用户允许。

**回滚点 B0**：尚未切分支；任何问题直接停止，`main` 未变化。

## 3. Bootstrap 子任务：当前目录创建并切换分支

- [ ] 从 ccs peeled commit 创建：

```bash
git switch -c agent-switch-ccs 8d1b3306d09a27b9d8fc29694791d8421aba5f93
```

- [ ] 立即验证：

```bash
git rev-parse HEAD
git status --short --branch
# package.json / Cargo.toml / tauri.conf.json 三处应为 3.16.5
```

- [ ] 验证工作树受控文件与 `git ls-tree` 一致，无旧 agent-switch 产品文件残留。
- [ ] 可选创建本地基线 tag（需用户同意，因为会改 refs）：`agent-switch-ccs-baseline` → `8d1b3306...`。

**回滚点 B1**：`git switch main`；若创建错误分支且无提交，确认后删除本地 `agent-switch-ccs`。不执行 `reset --hard`/`clean`。

## 4. Bootstrap 子任务：原样 ccs 基线验证

完整命令与预期见 `research/ccs-v3.16.5-validation-gates.md`。

### 4.1 来源与工具链

- [ ] Gate 0：HEAD/干净度/三处版本一致。
- [ ] Gate 1：Node 20、pnpm 10.12.3、Rust 1.95 MSVC、rustfmt/clippy、VS C++、WebView2、VBSCRIPT。
- [ ] 缺工具链时按用户授权安装/切换；记录实际版本。

### 4.2 官方 CI 等价门

- [ ] `pnpm install --frozen-lockfile`
- [ ] 安装后确认 lockfile/tracked 文件无漂移。
- [ ] `pnpm typecheck`
- [ ] `pnpm format:check`
- [ ] `pnpm test:unit`
- [ ] `pnpm build:renderer`
- [ ] `cargo fmt --check --manifest-path src-tauri/Cargo.toml`
- [ ] `cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings`
- [ ] `cargo test --locked --manifest-path src-tauri/Cargo.toml`
- [ ] `cargo check --locked --manifest-path src-tauri/Cargo.toml`

### 4.3 Windows build

- [ ] 先执行 `pnpm tauri build --no-bundle`，证明 release executable 可编译；记录结果。
- [ ] 完整原样 `pnpm tauri build`：若 updater 私钥不可用，明确记录“完整 MSI/updater artifact 由凭据阻塞”；不得改配置绕过。
- [ ] 若有安全测试 key，区分“artifact 生成链通过”与“原 ccs 内置公钥运行时签名不匹配”；不声称完成 updater E2E。
- [ ] 原样安装/启动 smoke 仅在隔离 Windows 用户/VM/Sandbox 做，防止读写真实 `~/.cc-switch`。

- [ ] 将每条命令、exit code、摘要、阻塞写入 bootstrap 子任务 check/research。

**回滚点 B2**：纯 ccs 起点无代码提交；失败不修 ccs 产品代码，先判断环境 vs 上游基线缺陷并回到规划。

## 5. Bootstrap 子任务：迁入 Trellis

### 5.1 文件迁入

- [ ] 从 `main` 恢复当前 Trellis 0.6.6 的 tracked 工作流文件（精确路径清单先 review）。
- [ ] 从 bootstrap 备份恢复未跟踪 audit/archive/本父任务。
- [ ] 合并 `.gitignore`：保留 ccs 规则，追加 Trellis 平台目录/runtime 忽略规则；不直接用旧文件覆盖。
- [ ] 不恢复任何旧产品文件：`src/`、`src-tauri/`、`package*.json`、`docs/release.md` 等。
- [ ] 检查 staged diff 只含预期 Trellis/bootstrap 内容。

### 5.2 平台生成与规范刷新

- [ ] 运行项目支持的 Trellis init/update，重建被忽略的 `.claude/` 等平台适配。
- [ ] 旧 `.trellis/spec/` 先标注为 legacy reference 或归档，禁止它直接指导 ccs 产品改动。
- [ ] 用 `trellis-spec-bootstrap` 基于 ccs v3.16.5 重建 frontend/backend/guides 规范索引；文档使用中文。
- [ ] 验证 `task.py current/list/create/start`、phase context、sub-agent dispatch 正常。

### 5.3 提交门

- [ ] 提交 1：Trellis bootstrap（仅工作流/任务/ignore）。
- [ ] 提交 2：ccs-based spec refresh（可与提交 1 分开审查）。
- [ ] 若用户未授权 commit，则停下汇报，不能把后续产品工作叠在未审查的 bootstrap diff 上。

**回滚点 B3**：回退 Trellis/bootstrap 提交即可回到纯 ccs commit；`main` 不受影响。

## 6. Claude-only 裁剪子任务

以 `research/ccs-v3.16.5-trim-map.md` 为依据，逐批执行，每批先查引用后删。

### 6.1 冻结保护清单

- [ ] 为以下 Claude 复用模块建立/保留测试，禁止误删：
  - `ProviderType::CodexOAuth`
  - `ProviderType::GitHubCopilot`
  - `ProviderType::OpenRouter`
  - `commands/codex_oauth.rs` / `commands/copilot.rs`
  - Codex OAuth / Copilot auth、账号、配额、模型映射及 Claude Provider 表单链
  - `streaming_responses.rs`
  - `transform_responses.rs`
  - Claude 上游引用的 Codex Chat/Copilot 协议转换闭包
- [ ] 固定 schema v11，不 drop 多应用列。

### 6.2 推荐批次

- [ ] 保留代理/热切换/格式转换/熔断故障转移/用量与请求日志/Claude Desktop gateway；裁剪后回归 Claude 主链路请求。
- [ ] 实现非 loopback 鉴权中间件：监听 `127.0.0.1`/`localhost`/`::1` 放行，其余地址对所有转发端点校验本地 Bearer token，缺失或不匹配返回 401。
- [ ] 前端保存非 loopback 监听地址前增加并测试持久化风险确认；默认仍为 `127.0.0.1`。
- [ ] 批次 C2：`App.tsx` / AppSwitcher / VALID_APPS 收缩到 Claude。
- [ ] 批次 C3：后端非 Claude commands/services/config modules。
- [ ] 批次 C4：非 Claude proxy client adapters（保留上游 ProviderType/Responses translation）。
- [ ] 批次 C5：Sessions/MCP/Prompts/Skills/Deep Link/Usage 单应用化。
- [ ] 批次 C6：Rust module declarations、invoke handler、前端 exports、tests 残留清理。
- [ ] 保持 Claude `provider.settings_config` 为完整 settings.json 任意 JSON 快照；不迁入旧 agent-switch 的 `meta.snapshot`/端点引用数据模型。
- [ ] 回归三层切换：切出 live 回填并剥 Common Config → 目标 `settings_config` 叠加 Common Config → sanitizer 后整份写 live。
- [ ] 增加未知字段往返测试，覆盖顶层未知对象、`env` 非连接键、hooks/permissions 类嵌套结构；结构化表单修改已知字段后不得删除其余键。
- [ ] 逐项审核 `claudeProviderPresets.ts`：每个保留模板记录官方站点/官方兼容文档依据；删除聚合/中转、`isPartner`/`primePartner`/`partnerPromotionKey` 展示链及所有 aff/ref/utm/ccswitch 跟踪参数。
- [ ] 保留 Provider 裸 JSON 自定义入口并增加回归测试，确保精简预设不限制任意完整 `settings_config`。

每批验证：

```text
pnpm typecheck
pnpm format:check
pnpm test:unit
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked
cargo check --locked
```

- [ ] 每批做 app ID/path/command/reference 残留搜索。
- [ ] 每批独立提交，不合并成大删除提交。

**回滚点 Cn**：逐批 revert；schema 未改，无 DB rollback。

## 7. Windows + zh-CN + 品牌内容裁剪子任务

- [ ] 删除 en/ja/zh-TW locale，i18n 固定 zh；删除语言切换 UI。
- [ ] 删除 `flatpak/`、Linux/macOS/移动端专属资产/代码/CI。
- [ ] 删除 OpenClaw 专属 `workspace` view 与 `commands/workspace.rs`。
- [ ] 保留本地 DB 备份、WebDAV/S3 v2、自动同步、`db.sql`/`skills.zip` artifact 构建与恢复；裁剪后验证 snapshot 仅含 Agent Switch Claude-only 数据。
- [ ] 将 WebDAV/S3 remote root、manifest 产品身份与提示文案从 ccs 改为 Agent Switch，避免与 CC Switch 远端目录混用。
- [ ] 首次启用远端同步前增加并测试持久化风险确认：明确 `db.sql` 无客户端内容加密、可能含 Provider API token、远端管理员可读取。
- [ ] release matrix 首期只留 Windows x86_64；ARM64 不作为门。
- [ ] 删除/替换 ccs sponsor、affiliate、Funding、社区 issue/PR/stale 内容。
- [ ] 保留并审查 `LICENSE`；添加“基于 CC Switch v3.16.5 修改”的来源说明。
- [ ] README/用户文档只维护中文。
- [ ] 按同一质量门验证并独立提交。

## 8. Agent Switch 身份子任务

### 8.1 身份与版本

- [ ] `productName = Agent-Switch`
- [ ] `identifier = com.agent-switch.app`
- [ ] npm/Cargo/lib 名切换为 agent-switch / agent_switch_lib；修全部 Rust/JS 引用。
- [ ] package/Cargo/Tauri 三处版本同步为 `0.3.0`。
- [ ] Windows AUMID、窗口/托盘/关于页、User-Agent、导出 header、日志名、README、资产全面替换。

### 8.2 数据根

- [ ] 把 `get_app_config_dir()` 根改为 `~/.agent-switch`，派生：
  - `agent-switch.db`
  - `backups/`
  - `skills/`、`skill-backups/`
  - `logs/agent-switch.log`
  - `crash.log`
- [ ] 删除/替换 `CC_SWITCH_TEST_HOME`；新增统一 Agent Switch test-home 常量。
- [ ] 禁止 compatibility fallback 到 `~/.cc-switch` 或旧 Agent Switch DB。
- [ ] 新 HOME 测试：首启只创建 `~/.agent-switch`；预置诱饵 `~/.cc-switch`/旧 DB 保持字节不变。
- [ ] 同一新 HOME 中预置含未知嵌套字段的 `~/.claude/settings.json`：验证 import-before-seed 创建 current `default` 全文快照，首次切换后可回填恢复；无效 JSON/代理占位输入必须阻止静默覆盖。

### 8.3 Deep Link

- [ ] Tauri scheme、runtime listener、parser、frontend、source_protocol、tests 全改 `agentswitch://`。
- [ ] 系统不注册 `ccswitch://`。
- [ ] 是否接受应用内粘贴 `ccswitch://` 另立明确开关/测试；若无需求则不做。

### 8.4 MSI 身份

- [ ] 在旧 Agent Switch 身份上用 `tauri inspect wix-upgrade-code` 获取旧默认 UpgradeCode。
- [ ] 在新配置显式写 `bundle.windows.wix.upgradeCode`。
- [ ] 决定保留 ccs per-user WiX 模板或旧 Agent Switch installer 形态；写明取舍。
- [ ] 在隔离 Windows 环境测试：旧 Agent Switch → 0.3.0 升级；CC Switch 保持独立可共存。

## 9. Updater/发布子任务

- [ ] 内置现有 Agent Switch updater 公钥（先核对与安全私钥配对）。
- [ ] endpoint 指向 `XM-Chen/agent-switch/releases/latest/download/latest.json`。
- [ ] GitHub release workflow 仅构建 Windows x86_64，移除 ccs 官方 secrets/资产名/多平台矩阵。
- [ ] 私钥从安全路径/secret 注入；绝不读取/提交明文到仓库或日志。
- [ ] 构建 MSI + `.sig`。
- [ ] 生成 `latest.json`：version `0.3.0`，`windows-x86_64.url`，`signature` = `.sig` 文件内容。
- [ ] 用已安装旧 Agent Switch 验证 check → download → signature verify → install/restart。
- [ ] 不执行真实 push/release，除非用户对该外部操作另行确认。

## 10. 最终回归子任务

### 10.1 静态残留门

- [ ] 无产品身份残留：`CC Switch` / `com.ccswitch.desktop` / `~/.cc-switch` / ccs updater endpoint / system `ccswitch` registration（LICENSE/来源说明白名单除外）。
- [ ] 无被删除客户端的 UI route、invoke command、live projection、session provider、MCP enable 分支（Claude 上游 ProviderType 白名单除外）。
- [ ] 无 en/ja/zh-TW locale 和 macOS/Linux release target。
- [ ] schema 仍为 v11，迁移测试全绿。

### 10.2 质量门

- [ ] frozen pnpm install 无 lockfile 漂移。
- [ ] typecheck / format / all frontend tests / renderer build。
- [ ] Rust fmt / clippy -D warnings / all tests / check。
- [ ] `pnpm tauri build --no-bundle`。
- [ ] 凭据可用时完整 Windows MSI/updater artifacts。

### 10.3 Windows 端到端行为

- [ ] Provider 创建/编辑/普通切换/Common Config/backfill。
- [ ] Proxy start/stop/takeover/hot switch/failover/format translation。
- [ ] Usage dashboard/请求日志/Token/成本。
- [ ] MCP、Prompts、Skills 各自 CRUD + Claude live 投影。
- [ ] Sessions 浏览/搜索/恢复。
- [ ] `agentswitch://` provider/prompt/mcp/skill 导入。
- [ ] 新空 HOME 数据隔离。
- [ ] 旧 Agent Switch 升级、与 CC Switch 共存。
- [ ] 自有 updater 端到端（仅发布授权时）。

## 11. 完成/提交/归档

- [ ] 每个子任务跑 `trellis-check`，必要时 `verify` 驱动真实应用行为。
- [ ] 最后一轮做父任务跨子任务集成 review。
- [ ] 更新 ccs-based `.trellis/spec/`，沉淀上游同步、单应用裁剪、数据/身份隔离、updater 约定。
- [ ] 用户授权后按逻辑批次提交；不把 Trellis bootstrap、批量删除、身份改造、updater 混成一个 commit。
- [ ] 未授权不 push；若授权，首次推送 `origin/agent-switch-ccs`，不改 `origin/main`。
- [ ] 所有验收完成后 finish/archive 子任务，再归档父任务。

## 12. 首个执行停点

用户批准本规划后，**只启动 `ccs-baseline-bootstrap` 子任务**，完成：

1. 保护未跟踪 Trellis 内容；
2. 当前目录切换到 `agent-switch-ccs`；
3. 验证纯 ccs v3.16.5 基线；
4. 迁入 Trellis 并刷新 spec。

完成并 review 该地基后，才进入大规模功能裁剪。
