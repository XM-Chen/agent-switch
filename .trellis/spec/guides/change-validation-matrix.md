# 变更验证矩阵

## 通用门禁

每个产品代码批次至少执行：

```bash
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo check --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri build --no-bundle
```

依赖未变可不重复 install；改 lockfile 必须先 `pnpm install --frozen-lockfile` 验证无漂移。

## 按变更类型追加验证

| 变更 | 附加检查 |
|---|---|
| AppType/AppId 裁剪 | Rust enum/FromStr/all、前端 APP_IDS/VALID_APPS、commands/handler、MCP/Skills/Sessions/Deep Link、DB 行、残留搜索 |
| Provider 表单/切换 | 未知 JSON 字段往返、Common Config merge/strip、外部 live 回填、takeover、失败回滚 |
| 首次启动 | 新 HOME、已有/缺失/无效 Claude settings、import-before-seed、`~/.cc-switch` 诱饵不变 |
| Proxy/router | loopback/非 loopback 鉴权矩阵、每类 route、stream 首块、failover、日志脱敏 |
| OAuth/Copilot | 登录、账号绑定、refresh 并发、配额、模型、Claude request；确认无独立 Codex 客户端回流 |
| DB/schema | memory DB、升级、未来版本拒绝、预迁移备份、外键/索引 |
| WebDAV/S3 | 风险确认、artifact 内容/manifest/remote root、冲突/恢复、安全备份、无 secret 日志 |
| 身份/路径 | 全仓旧身份扫描（允许 LICENSE/来源说明）、新 HOME 隔离、Windows AUMID/日志/DB/backup |
| Deep Link | 系统只注册 agentswitch，四类导入、非法 scheme、source_protocol |
| Updater | 三处版本、自有 endpoint/pubkey、MSI/.sig/latest.json、安装 E2E（有凭据时） |
| i18n/中文 | UI 无语言切换、只加载 zh、无 en/ja/zh-TW locale/key 引用 |
| Windows-only | cfg/依赖/CI/release matrix、Windows Rust 全门、no-bundle build |

## 静态残留门

最终扫描并分类，不盲目要求字符串绝对为零：

- ccs 身份：`CC Switch`、`cc-switch`、`com.ccswitch.desktop`、`~/.cc-switch`、`ccswitch://`、官方 updater endpoint；
- 删除客户端：Codex/Gemini/OpenCode/OpenClaw/Hermes/Claude Desktop 的 AppId、route、live、session、MCP enable；
- 多语言：en/ja/zh-TW locale 与语言切换；
- 平台：macOS/Linux/Flatpak/release target。

允许列表必须明确：MIT/来源说明、GitHub Copilot/Codex OAuth/OpenRouter/Responses 上游、必要协议模型名。每个命中都要说明为何保留。

## 行为验收

最终在隔离 Windows 环境验证：

- 安装/升级不覆盖 CC Switch；
- 首启只创建 `~/.agent-switch`，不读旧产品 DB；
- 现有 Claude live 全文保护导入；
- Provider CRUD/切换/Common Config/backfill；
- Proxy 直连/hot switch/failover/format/usage/log；
- Copilot/Codex OAuth 作为 Claude 上游；
- MCP/Prompts/Skills/Sessions；
- `agentswitch://` 四类资源；
- 本地备份与 WebDAV/S3；
- 有发布授权时 updater E2E。

## 结果记录

每条命令记录 exit code、摘要和真实阻塞。不得把 skipped 写成 passed，不得用“环境问题”替代具体原因。原样基线结果参考 bootstrap 的 `research/r3-validation-results.md`，后续批次必须区分“已知基线失败”和“新回归”。
