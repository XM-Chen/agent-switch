# 项目与工具链约定

## 技术栈

ccs v3.16.5 是单仓库单包桌面应用：

- 前端：React 18、TypeScript、Vite、Tailwind 3、TanStack Query、i18next、Vitest；
- 后端：Rust 2021、Tauri 2.8.2、SQLite/rusqlite、Tokio、Axum；
- 包管理：pnpm lockfile；
- Windows 构建：Tauri + MSVC，首期 x86_64。

证据：`package.json:1-94`、`src-tauri/Cargo.toml:1-113`。

## 版本来源要区分

| 项 | 值 | 含义 |
|---|---|---|
| `.node-version` | 22.12.0 | 本地版本提示 |
| GitHub CI Node | 20 | 上游可复现 CI 环境 |
| pnpm | 10.12.3 | 上游 CI 固定版本 |
| Cargo `rust-version` | 1.85.0 | MSRV |
| `rust-toolchain.toml` | 1.95 | 实际固定工具链 |

不能把 MSRV、toolchain pin 和 CI runtime 混写成一个版本。当前本机 Node 22.19 已完成基线验证，但最终 CI 应明确固定版本。

## 有效命令

```bash
pnpm install --frozen-lockfile
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

仓库不存在 `pnpm lint` script；CONTRIBUTING 中该描述陈旧。locale 实际位于 `src/i18n/locales/`，不是旧文档路径。以 package/source/CI 为准。

## Git 与提交边界

- `main` 保留旧 Agent Switch，不重写；当前分支以 ccs release commit 为根。
- 未经明确授权不 commit、push、改默认分支、打 tag 或发布。
- 用户授权 commit 后按逻辑边界：Trellis bootstrap、spec refresh、每个裁剪批次、身份、updater 独立提交。
- 不把大规模删除、格式化、身份改名和行为新增混为一笔。
- 定期手动同步 ccs 上游：固定 release SHA → 评估 → merge/cherry-pick → 完整回归。

## 来源与许可

- 保留 MIT `LICENSE` 与 Jason Young 原版权；
- README/关于页注明“基于 CC Switch v3.16.5 修改”；
- 删除 ccs 商业合作/赞助/联盟内容不等于删除许可归属。

## 秘密与发布

- updater 私钥、API token、OAuth 凭据绝不入库、日志或 spec；
- `.gitignore` 忽略 `*.key`、`*.key.pub`（公钥内容应进入 Tauri 配置而非散落文件）、生成 `latest.json`；
- 完整 bundle 需安全注入 `TAURI_SIGNING_PRIVATE_KEY`，无密钥时只执行 `--no-bundle`，不得改 updater 配置绕过。
