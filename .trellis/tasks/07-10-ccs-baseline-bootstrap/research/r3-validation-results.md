# R3 原样 ccs v3.16.5 基线验证结果

> 执行时间：2026-07-10。执行人：Claude（agent-switch-ccs 分支）。
> 验证位置：`E:/SynologyDrive/git_files/agent-switch`，HEAD = `8d1b3306d chore(release): v3.16.5`。
> 工作树状态：产品代码（`src/`、`src-tauri/src/`）零未提交改动；唯一改动为 `.trellis/` 规划文档。即所有 pass/fail 均来自 ccs 原样基线，非本分支引入。
> 基线判定：`git diff` 三个失败文件（`misc.rs`/`codex_history_migration.rs`/`services/provider/mod.rs`）均无改动；`misc.rs` 的 codex 升级逻辑来自上游 commit `8f484c54c`（PR #4782）。

## Gate 0 — 来源/干净度/版本一致性

| 项 | 结果 | 证据 |
|---|---|---|
| HEAD | ✅ | `8d1b3306d chore(release): v3.16.5`（= 官方 tag `v3.16.5^{}` peeled commit） |
| 工作树产品代码 | ✅ 无改动 | `git status --short -- src src-tauri/src` 返回空 |
| `package.json` version | ✅ | 3.16.5 |
| `src-tauri/Cargo.toml` version | ✅ | 3.16.5 |
| `src-tauri/tauri.conf.json` version | ✅ | 3.16.5 |
| 嵌套 `.git` | ✅ 无 | 单一仓库根 |
| MIT LICENSE + Jason Young 版权 | ✅ 保留 | 未触及 |

## Gate 1 — 工具链版本

| 工具 | 官方要求 | 实际 | 状态 |
|---|---|---|---|
| Node | 20 | 22.19.0 | ⚠️ 与官方不同（CI 等价门仍通过，见下） |
| pnpm | 10.12.3 | 10.12.3（`npm i -g pnpm@10.12.3` 安装；corepack EPERM 故改 npm） | ✅ |
| Rust | 1.95（MSRV 1.85） | 1.95.0 | ✅ |
| rustfmt/clippy | 随 toolchain | 1.95.0 | ✅ |
| VS C++ / WebView2 | Windows 构建需要 | 已装（build 链验证中） | — |
| updater 私钥 | 完整 bundle 需要 | **缺失** | ❌ 阻塞完整 MSI/.sig（见 Gate 7） |

## Gate 2 — 前端 CI 等价门

| 命令 | exit | 结果 | 说明 |
|---|---|---|---|
| `pnpm install --frozen-lockfile` | 0 | ✅ | lockfile 无漂移 |
| `pnpm typecheck` | 0 | ✅ | `tsc --noEmit` 通过 |
| `pnpm format:check` | 0 | ✅ | `prettier --check` 通过 |
| `pnpm test:unit` | 1 | ⚠️ 4 failed | 见下方“前端失败” |
| `pnpm build:renderer` | 0 | ✅ | `vite build` 通过 |

### 前端失败（4，全部 OpenClaw 相关，ccs 上游既存）

- 文件：`tests/integration/App.test.tsx`
- 失败根因：`getByText("switch-openclaw")` 在多 App 渲染中匹配到多个 DOM 元素（AppSwitcher 同时渲染了开关按钮与列表项）。
- 性质：ccs v3.16.5 上游测试缺陷，非本分支引入，非 flaky（4 次稳定失败）。
- 处置：按 R3 验证阶段不修产品代码；OpenClaw 裁剪批次（子任务 `ccs-claude-only-trim`）会删除 `workspace` view 与 OpenClaw 入口，届时这些测试一并删除。

## Gate 3 — Rust CI 等价门

| 命令 | exit | 结果 | 说明 |
|---|---|---|---|
| `cargo fmt --check --manifest-path src-tauri/Cargo.toml` | 0 | ✅ | 格式通过 |
| `cargo check --locked --manifest-path src-tauri/Cargo.toml` | 0 | ✅ | 编译通过（10 warnings，非 `-D warnings` 不阻断） |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings` | 101 | ❌ 13 errors | 见下方“Clippy 失败” |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml` | 101 | ❌ 8 failed / 1705 passed | 见下方“Rust 测试失败” |

### Clippy 失败（13 errors，Windows 平台死代码，ccs 上游既存）

ccs 官方 CI 在 Linux 跑 clippy，以下代码被 `cfg(target_os="linux"/"unix")` 调用点引用，Linux 下非死代码；Windows 编译时调用点被 cfg 排除 → 函数变死代码 → `-D warnings` 报错。

| 类型 | 位置 | 项 |
|---|---|---|
| unused import | `src/settings.rs:3` | `std::io::Write` |
| dead_code | `src/commands/misc.rs:1072` | `fallback_user_shell` |
| dead_code | `src/commands/misc.rs:1080` | `valid_user_shell_path` |
| dead_code | `src/commands/misc.rs:1103` | `is_executable_file` |
| dead_code | `src/commands/misc.rs:1108` | `get_user_shell` |
| dead_code | `src/commands/misc.rs:1116` | `build_exec_line` |
| dead_code | `src/commands/misc.rs:1135` | `build_provider_command_line` |
| dead_code | `src/commands/misc.rs:1155` | `provider_command_flag_for_shell` |
| dead_code | `src/commands/misc.rs:1163` | `build_final_shell_cd_command` |
| dead_code | `src/commands/misc.rs:3255` | `shell_single_quote` |
| needless_return | `src/commands/misc.rs:641` | — |
| needless_return | `src/commands/misc.rs:2734` | — |
| needless_return | `src/commands/settings.rs:250` | — |

- 8 个 shell 辅助函数无单独 cfg 门控，唯一调用点在 `launch_linux_terminal`（`#[cfg(target_os="linux")]`）与 macOS 终端启动代码。
- 性质：上游跨平台 cfg 缺陷，非本分支引入。
- 处置：按 R3 验证阶段不修。这些 shell 启动代码属“非 Claude 客户端集成/非 Windows”裁剪范围，将在 `ccs-windows-zh-trim` 与 `ccs-claude-only-trim` 批次删除（删调用链后这些函数随之消失，或显式加 `#[cfg(unix)]`）。

### Rust 测试失败（8，全部 ccs 上游 Windows 既存）

| 类别 | 数量 | 测试 | 根因 |
|---|---|---|---|
| Codex 升级命令（`anchored_upgrade_windows`） | 6 | `npm_windows_default_branch`、`windows_no_sibling_uses_cli_update_without_package_fallback`、`volta_windows_uses_volta_install`、`pnpm_windows_uses_pnpm_add`、`windows_full_batch_line_for_percent_path_uses_quadruple_escape`、`windows_path_with_space_is_double_quoted` | 测试期望 `codex.cmd update \|\| <fallback>`（先试 codex 自更新再回退），实现只产出 fallback。实现与测试在同一上游版本 `8f484c54c`（PR #4782）已不一致 |
| codex_history 迁移 | 1 | `codex_history_migration::tests::config_sqlite_home_takes_precedence_over_codex_sqlite_home_env` | 环境变量优先级：实际 `env-sqlite-home` 胜出，期望 `config-sqlite-home` 胜出 |
| Claude Desktop provider 同步 | 1 | `services::provider::tests::update_current_claude_desktop_provider_syncs_profile_when_proxy_takeover_is_active` | 代理 takeover 下 profile 同步断言不符 |

- 全部 8 项失败文件无本分支改动（`git diff HEAD` 空）。
- Linux CI 不跑 `anchored_upgrade_windows` 子模块（`cfg(target_os="windows")`），故上游未发现。
- 处置：按 R3 验证阶段不修。Codex 升级命令与 codex_history 迁移属“独立 Codex 客户端”裁剪范围（D25：保留 OAuth 上游但删独立客户端），将在裁剪批次删除；Claude Desktop sync 失败需在保留模块回归子任务（`ccs-retained-features-regression`）单独排查。

### 构建产物缓存冲突（已解决，非代码缺陷）

- 现象：首次 `cargo test` 报 `E0786 invalid metadata files for crate cc_switch_lib` / `crate X required in rlib format`。
- 根因：同一 target 目录交替运行 `cargo check`（dev profile，只产 `.rmeta`）与 `cargo test`（需 `.rlib`）导致产物交叉污染——cargo 已知行为，非代码问题。
- 解决：`cargo clean`（全量，清 45GB）后重跑 `cargo test` 成功编译，E0786 消失，进入真实测试断言失败阶段。
- 教训：后续 Rust 门禁应统一用同一 profile 顺序，或分用不同 `CARGO_TARGET_DIR`。

## Gate 4 — Windows 构建

| 命令 | exit | 结果 |
|---|---|---|
| `pnpm tauri build --no-bundle` | 0 | ✅ 通过。release 编译 17m40s，产物 `src-tauri/target/release/cc-switch.exe`（30,679,040 字节）。10 warnings（同 Gate 3 check，非阻断） |
| `pnpm tauri build`（完整 MSI + .sig） | — | 阻塞：updater 私钥缺失（见 Gate 6）。按 R3 未改 `createUpdaterArtifacts`/updater 配置绕过 |

`--no-bundle` 证明 release 可执行文件可编译；完整 MSI + `.sig` + `latest.json` 端到端待 `ccs-updater-release` 子任务生成自有密钥对。

## Gate 5 — 原样安装/启动 smoke

- 状态：**未执行**。
- 原因：本机存在真实 `~/.cc-switch`，原样运行会污染。需隔离环境（独立 OS 用户 / VM / Windows Sandbox）。
- 处置：首期不强制执行；身份改造后（`~/.agent-switch`）在隔离环境做端到端 smoke。

## Gate 6 — 环境阻塞汇总（真实，非笼统）

1. **Node 22.19 vs 官方 20**：CI 等价门已通过，不阻塞验证；记录差异。
2. **corepack EPERM**：`corepack prepare pnpm@10.12.3` 在 `D:\nodejs\pnpm` 报 EPERM，改用 `npm i -g pnpm@10.12.3` 解决。
3. **updater 私钥缺失**：阻塞完整 MSI + `.sig` + `latest.json` 端到端。首期个人自用可先用 `--no-bundle` 验证 release 可执行文件；完整发布门待 `ccs-updater-release` 子任务生成自有密钥对。
4. **原样 smoke 隔离**：本机 `~/.cc-switch` 存在，需隔离环境，首期不执行。

## Gate 7 — 不得为绕过失败而修改的项（R3 红线，已遵守）

- ✅ 未修改 `createUpdaterArtifacts` / updater 公钥 / endpoint / bundle target / WiX 模板。
- ✅ 未修改任何产品代码以绕过 clippy/test 失败。
- ✅ 未在非隔离环境运行原样 smoke。

## 结论（AC4 真实记录）

- **通过**：来源完整性、版本一致性、前端 typecheck/format/build、Rust fmt/check、Rust 测试 1705/1713 通过。
- **原样基线既有失败（非本分支引入，按 R3 不在验证阶段修）**：
  - 前端 4 个 OpenClaw 测试（裁剪时随 OpenClaw 删除）。
  - Clippy 13 errors（Windows 死代码 + needless_return，裁剪时随非 Windows/Codex 客户端代码删除或加 cfg）。
  - Rust 8 测试失败（6 Codex 升级命令 + 1 codex_history + 1 Claude Desktop sync，裁剪时随独立 Codex 客户端删除；Claude Desktop sync 留待回归子任务）。
- **阻塞**：完整 MSI/updater artifact（updater 私钥缺失）；原样 smoke（隔离要求）。
- **基线可用性判定**：ccs v3.16.5 作为定制起点**可接受**——所有失败均可在既定裁剪/回归批次中消除，无需在验证阶段偏离 R3 修产品代码。可进入 Trellis 迁入 + spec refresh（步骤 5），随后功能裁剪。
