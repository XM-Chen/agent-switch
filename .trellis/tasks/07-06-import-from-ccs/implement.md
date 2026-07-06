# Implement Plan: 一键从本地 ccs 导入 Claude 上游渠道

> 配套：`prd.md`、`design.md`、`research/ccs-data-format.md`、`research/agent-switch-direct-provider-path.md`。
> 质量门（每步结束跑）：`cd src-tauri && cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib`，前端 `npm run build`（含 tsc）。

## 执行检查表（有序）

### 步骤 1：后端 importers 模块骨架
- [ ] 新建 `src-tauri/src/services/importers/mod.rs`：`pub mod ccs;`
- [ ] 在 `src-tauri/src/services/mod.rs` 加 `pub mod importers;`
- [ ] 新建 `src-tauri/src/services/importers/ccs.rs`：先写 ccs 数据结构反序列化类型（`CcsConfig`/`CcsProvider`，对齐 research 文件第 1-2 节的 `ProviderManager`/`Provider`，字段 `id/name/settingsConfig/websiteUrl`）。
- [ ] **验证**：`cargo check` 通过。
- **风险**：无，纯类型定义。

### 步骤 2：detect 逻辑（ccs 解析 + 本地比对）
- [ ] `ccs.rs` 实现 `pub fn detect(db, config_path: Option<PathBuf>) -> Result<DetectResponse, String>`：
  - 解析 `~/.cc-switch/config.json`（默认路径 `dirs::home_dir().join(".cc-switch/config.json")`）；不存在 → `found=false`。
  - 解析 JSON 为 `CcsConfig`；失败 → `Err`。
  - 对每个 ccs provider 提取 `env.ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`/`ANTHROPIC_MODEL`（用 `serde_json::Value` 导航，缺键容忍）。
  - base_url 缺失/为空 → `importable=false, warning="无 base_url（官方登录渠道，无上游端点，无法导入）"`。
  - 查本地 `providers::list_by_app(db, "claude-code")`，按 `meta.original_id` 匹配 → `already_imported`；按 `name` 匹配 → `conflict` + 调 `resolve_unique_name` 算 `imported_name`。
  - 返回 `DetectResponse { config_path, found, providers: Vec<DetectItem> }`。
- [ ] 写单测：mock ccs config.json（用 tempdir 写文件）+ 内存 DB，覆盖 found=false、正常项、空 env 项、conflict、already_imported 五种情况。
- [ ] **验证**：`cargo test --lib importers::ccs`。
- **风险**：`dirs::home_dir()` 在测试环境可能为 None——detect 接受 `config_path` 参数正是为了测试注入路径。本地比对依赖 `providers::list_by_app`，内存 DB + migrations 已有先例（见 `providers.rs` 测试 setup）。

### 步骤 3：import 逻辑（建 endpoint + 建 provider + 回滚）
- [ ] `ccs.rs` 实现 `pub fn import(db, crypto, items: Vec<ImportItem>) -> Result<ImportResponse, String>`：
  - 对每个 item：重新读 ccs config 按 `original_id` 定位；找不到 → `skipped`。
  - 提取 base_url/api_key/model；base_url 空 → `skipped`。
  - 二次校验 `already_imported`（meta.original_id 已存在）→ `skipped, reason="已导入过"`。
  - 建 endpoint：`endpoints::create(NewEndpoint { id: uuid, name: imported_name, base_url, protocol_type: "anthropic", api_key_encrypted: encrypt_api_key(...), auth_mode: "api_key", account_id: None, priority: 0, extra_json: None })`。
    - crypto 不可用 + 有 api_key → 该项 `errors, message="系统凭据管理器不可用，无法保存凭据"`，跳过 endpoint 创建。
  - 建 provider：`providers::create(NewProvider { id: uuid, app_type: "claude-code", name: imported_name, mode: "direct", settings_config: json!({"endpoint_id": ep_id, "model": model?}).to_string(), category: Some("custom"), sort_index: Some(next_sort_index), notes: None, meta: json!({"imported_from":"ccs","original_id":<ccs_id>,"website_url":<website_url>}).to_string() })`。
  - provider 创建失败 → `endpoints::delete(ep_id)` 回滚 + `errors`。
- [ ] `resolve_unique_name` 函数（design 第 4 节算法）+ 单测。
- [ ] 写单测：mock ccs config + 内存 DB + mock crypto（CryptoService 测试构造），覆盖正常导入、conflict 重命名、空 env 跳过、二次导入幂等、crypto 不可用降级。
- [ ] **验证**：`cargo test --lib importers::ccs`。
- **风险**：crypto 在测试里如何构造——查 `crypto.rs` 是否有测试用 master key 构造法（`CryptoService::new(key)` 接受 master key，测试可固定 key）。endpoint+provider 原子性：单项内 provider 失败要回滚 endpoint，注意 delete 失败不要掩盖原错误。

### 步骤 4：HTTP 端点接线
- [ ] `src-tauri/src/http/api/providers.rs` 加两个路由：
  - `POST /api/providers/import-ccs/detect` → `detect_handler`：调 `importers::ccs::detect(&state.db, None)`，返回 `Json<DetectResponse>`。
  - `POST /api/providers/import-ccs` → `import_handler`：解析 `ImportRequest`，调 `importers::ccs::import(&state.db, &state.crypto, req.items)`，返回 `Json<ImportResponse>`。
- [ ] 在 `routes()` 注册（注意固定段 `/import-ccs/...` 必须先于 `/{id}` 注册，参考现有 `/reorder` 先例 `providers.rs:104`）。
- [ ] 请求/响应结构体加 `Serialize`/`Deserialize`。
- [ ] **验证**：`cargo check` + `cargo clippy`。
- **风险**：路由顺序——`/{id}` 会吞掉 `/import-ccs`，必须先注册固定段。参考现有 `/reorder` 写法。

### 步骤 5：前端 API 层
- [ ] `src/lib/api.ts` 新增 `ccsImportApi`：`detect()` 调 `POST /api/providers/import-ccs/detect`，`import(items)` 调 `POST /api/providers/import-ccs`。
- [ ] 定义 TS 类型 `CcsDetectItem`/`CcsDetectResponse`/`CcsImportItem`/`CcsImportResponse`，对齐后端契约。
- [ ] **验证**：`npm run build`（tsc --noEmit 通过）。
- **风险**：无，纯类型 + fetch。

### 步骤 6：前端导入对话框
- [ ] 新建 `src/components/providers/ImportCcsDialog.tsx`：
  - 打开时调 `ccsImportApi.detect()`，loading/错误态。
  - `found=false` → 显示"未检测到 ccs 安装（~/.cc-switch/config.json 不存在）"。
  - 列表渲染：每项 checkbox + name + base_url + 状态标签（已导入/冲突→导入后名/不可导入→warning）。`importable=false` 与 `already_imported=true` 默认不勾选。
  - 勾选确认后调 `ccsImportApi.import(items)`，展示结果（created/skipped/errors）。
  - 成功后 invalidate `['providers']` query。
- [ ] `src/pages/ProvidersPage.tsx` 在"添加 provider"按钮旁加"从 ccs 导入"按钮，控制对话框 open 态。
- [ ] **验证**：`npm run build` + `npm run test`（若新组件有可测纯逻辑则加 vitest，否则手动验证）。
- **风险**：UI 复杂度中等。对齐现有 ProviderForm 的样式与交互模式（深色模式、banner 反馈）。

### 步骤 7：端到端验证
- [ ] 本机造一个测试用 `~/.cc-switch/config.json`（含 1 个正常项 + 1 个空 env 项）。
- [ ] `npm run tauri dev` 启动应用，切换器页点"从 ccs 导入"，确认预览列表正确。
- [ ] 勾选正常项导入，确认切换器页出现新 provider；点切换，确认 `~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN` 被写入。
- [ ] 重复导入，确认已导入项被标注且不重复创建。
- [ ] 确认 `~/.cc-switch/config.json` 未被修改。
- [ ] **验证**：手动 + 检查 DB（`sqlite3` 查 providers/endpoints 表的 meta 和 endpoint_id 关联）。

### 步骤 8：质量门收敛
- [ ] `cd src-tauri && cargo fmt`（修复格式）+ `cargo fmt --check`（确认无违规）。
- [ ] `cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib`。
- [ ] `npm run build`（前端 tsc + vite）。
- [ ] 全绿后进入 finish 阶段（spec 更新 + commit）。

## 验证命令汇总

```bash
# 后端
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib importers::ccs    # 步骤 2/3 的单测
cargo test --lib                    # 全量

# 前端
npm run build   # tsc --noEmit + vite build
npm run test    # vitest（若有新前端单测）
```

## 风险文件 / 回滚点

- **新文件为主**：`services/importers/{mod,ccs}.rs`、`components/providers/ImportCcsDialog.tsx`——失败可整文件删除回滚，不影响现有功能。
- **改动现有文件**：
  - `http/api/providers.rs`（加路由）：回滚点——新路由单独注册，删除即可。
  - `services/mod.rs`（加 `pub mod importers`）：一行删除。
  - `lib/api.ts`（加 ccsImportApi）：追加内容，删除即可。
  - `pages/ProvidersPage.tsx`（加按钮+对话框接入）：改动小，git revert 安全。
- **不动**：DAO 层、migrations、tool_takeover、切换路径、ccs 源文件。

## Review Gate（task.py start 前）

- [ ] PRD 已收敛（无 TBD/重复事实/已解决的 Open Questions）。
- [ ] design.md 覆盖架构/契约/trade-off/回滚。
- [ ] implement.md 步骤可执行、验证命令明确。
- [ ] research 文件完整（两份已在 research/ 目录）。
- [ ] 用户 review 通过后再 `task.py start`。
