# Implement：ccs 基线 bootstrap

> 详细步骤与命令见父任务 `07-10-ccs-baseline-migration/implement.md` 第 2–5 节及 `research/ccs-v3.16.5-validation-gates.md`。本文件是执行清单。未经用户批准不切分支/不改代码。

## 执行前

- [x] 用户批准父任务规划与本子任务。
- [x] 复核 `main` HEAD、`origin/main`、工作树状态与 `v3.16.5^{}`=`8d1b3306…`。
- [x] 确认创建分支前 `agent-switch-ccs` 不存在，tracked 无未提交产品改动。

## 步骤 1：保护（回滚点 B0）

- [x] 外部 bootstrap 备份 `.trellis-upgrade-audit.json`、`.trellis/archive/`、`.trellis/tasks/07-10-ccs-baseline-migration/`、本子任务目录。
- [x] 写 manifest + SHA-256，校验可读。

## 步骤 2：切换（回滚点 B1）

- [x] `git switch -c agent-switch-ccs 8d1b3306d09a27b9d8fc29694791d8421aba5f93`
- [x] 校验 HEAD/版本/`git ls-tree` 一致，无旧产品残留。
- [x] 不创建可选 `agent-switch-ccs-baseline` tag；以固定 SHA 作为唯一锚点。

## 步骤 3：原样验证（回滚点 B2）

- [x] Gate 0/1：来源、版本、工具链实测并记录。
- [x] 执行 `pnpm install --frozen-lockfile`、typecheck、format:check、test:unit、build:renderer；4 个 OpenClaw 既有测试失败已记录。
- [x] 执行 Rust fmt/clippy/test/check；clippy 13 errors、Rust 8 个既有 Windows 测试失败已记录。
- [x] `pnpm tauri build --no-bundle` 通过；完整 bundle 因 updater 私钥缺失而阻塞，未改配置绕过。
- [~] 隔离环境启动 smoke：**用户批准豁免（2026-07-10）**。本机存在真实 `~/.cc-switch`，原样 ccs 启动会读写它，风险不可接受；`--no-bundle` release executable 已证明可编译。原样启动 smoke 推迟到具备 Windows Sandbox/VM 时补跑，或并入后续「仅 Windows/中文裁剪」及身份改造（新 `~/.agent-switch` 空 HOME）阶段一并做隔离验证。不阻塞 bootstrap 收尾。
- [x] 每条命令 exit code、摘要、阻塞写入 `research/r3-validation-results.md`。

## 步骤 4：Trellis 迁入 + 规范刷新（回滚点 B3）

- [x] 从 `main` 恢复 tracked Trellis 0.6.6 runtime、框架规范、archive/workspace（精确 allowlist）。
- [x] 从备份恢复 audit/archive；当前父子任务保留本会话新版，不被旧备份覆盖。
- [x] 合并 `.gitignore`（ccs + Trellis 平台 + updater 私钥规则）。
- [x] 确认无旧产品源码进入 diff。
- [x] Trellis update、developer/current task、task.py list/current/create/start、phase/dispatch 验证通过。
- [x] 旧 spec 归档为 legacy；基于 ccs v3.16.5 重建 15 个中文 active spec。

## 步骤 5：提交门

- [x] 提交 1：Trellis bootstrap（仅 Trellis runtime/任务/审计/ignore）。
- [x] 提交 2：ccs-based spec refresh（active spec + legacy 归档）。
- [x] 用户在获知会产生两个独立提交后批准继续步骤 5。
- [x] 不 push、不改远程默认分支。

## 完成

- [x] `trellis-check` 核对 AC1–AC8；发现的暂存边界、CRLF、清单状态、workflow contract 引用均在提交前修复。
- [x] 汇报地基结果，后续由用户决定是否进入功能裁剪子任务。
