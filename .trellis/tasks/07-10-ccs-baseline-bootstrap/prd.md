# 建立 ccs v3.16.5 基线与 Trellis 迁入（地基）

> 父任务：`07-10-ccs-baseline-migration`。本子任务是地基批，被后续所有裁剪/身份子任务依赖。范围锁定为“安全切换 + 原样验证 + Trellis 迁入 + 规范刷新”，不做任何产品功能裁剪或身份改造。

## Goal

在当前 `E:/SynologyDrive/git_files/agent-switch` 目录，把 Git 从旧 `main` 安全切换到新分支 `agent-switch-ccs`（起点 = ccs 官方 `v3.16.5` peeled commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93`），验证该 ccs 基线原样可用，再把 Trellis 工作流与本任务迁入新分支并基于 ccs 刷新规范；全程保持旧 `main` 可恢复、未跟踪 Trellis 内容不丢失。

## Background

- 依据父任务决策 D1/D9/D10/D11/D20 与研究 `research/ccs-v3.16.5-validation-gates.md`、`research/branch-trellis-identity-plan.md`。
- 当前 `main` = `7e906685e`（ahead origin/main 1）。工作树未跟踪：`.trellis-upgrade-audit.json`、`.trellis/archive/`、`.trellis/tasks/07-10-ccs-baseline-migration/`（及本子任务目录）。
- ccs `8d1b3306…` 三处版本均 3.16.5，无 `.trellis`/`.claude`；与旧 agent-switch tracked `.trellis/**` 路径交集为零。
- ccs 工具链：CI 使用 Node 20 / pnpm 10.12.3，`rust-toolchain.toml` 固定 Rust 1.95（MSRV 1.85）/ Tauri 2.8.2；本机实测 Node 22.19、pnpm 10.12.3、Rust 1.95。完整 updater artifact 仍受私钥缺失阻塞。
- 旧 `.trellis/spec/` 描述旧 agent-switch 架构，迁入后不能直接指导 ccs 实现，必须先归档/刷新。

## Requirements

### R1 保护与切换

- 切换前把三类未跟踪内容复制到仓库外 bootstrap 备份并生成清单/哈希，验证可读后才动分支。
- 用 `git switch -c agent-switch-ccs 8d1b3306…` 在当前目录切换；不 `reset --hard`/`clean`；tracked 有未提交改动则停止交用户处理。
- `agent-switch-ccs` 已存在则停止调查，不覆盖。

### R2 原样验证

- 按研究文件的门执行；无法执行项记录真实阻塞，不伪装通过。
- 完整 MSI/updater 打包受私钥阻塞时，用 `pnpm tauri build --no-bundle` 验证 release executable，并显式标注完整 bundle 被凭据阻塞。
- 原样运行 smoke 仅在隔离环境进行，防止读写真实 `~/.cc-switch`。

### R3 Trellis 迁入与规范刷新

- 从 `main` 恢复 tracked Trellis 工作流文件；从 bootstrap 备份恢复未跟踪 audit/archive/父子任务。
- 合并 `.gitignore`（ccs 规则 + Trellis 平台忽略），不用旧文件覆盖。
- 不迁入任何旧产品源码（`src/`、`src-tauri/`、`package*.json`、`docs/release.md` 等）。
- 运行 Trellis init/update 重建被忽略的平台目录；验证 task.py/phase/dispatch 正常。
- 旧 spec 归档为 legacy reference 或基于 ccs v3.16.5 重建索引；未完成前禁止启动产品裁剪子任务。

### R4 提交边界

- 纯基线点 = 起点 commit，不新增提交。
- 提交 1：Trellis bootstrap（仅工作流/任务/ignore）。
- 提交 2：ccs-based spec refresh。
- 未获授权 commit 时停在可审查的 working-tree 状态汇报，不把后续工作叠加其上。
- 不 push、不改 `origin` 默认分支。

## Acceptance Criteria

- [x] AC1：bootstrap 备份含三类未跟踪内容且清单/哈希校验通过，切换后仍完好。
- [x] AC2：当前目录 HEAD = `8d1b3306…`，三处版本 3.16.5，工作树与 `git ls-tree` 一致，无旧 agent-switch 产品文件残留。
- [x] AC3：`git switch main` 能完整恢复旧 `main` 产品树与历史（`main` 未被重写）。
- [x] AC4：原样验证门按研究命令执行，结果（通过/阻塞）逐条记录；`pnpm tauri build --no-bundle` 已通过并生成 release executable。
- [x] AC5：Trellis 迁入后 task.py current/list/create/start、phase context、sub-agent dispatch 正常。
- [x] AC6：迁入提交只含 Trellis/任务/ignore，无旧产品源码；`.gitignore` 同含 ccs 与 Trellis 规则。
- [x] AC7：旧 spec 已归档为 legacy，active frontend/backend/guides 已基于 ccs v3.16.5 刷新；产品裁剪未在本子任务开始。
- [x] AC8：未 push、未改远程默认分支、未删除 bootstrap 备份。

## Out of Scope

- 任何产品功能裁剪（客户端/平台/语言/模块）。
- Agent Switch 身份、版本、数据目录、Deep Link、updater 改造。
- 数据迁移。
- push / release。

## Execution Defaults

- 不创建额外本地基线 tag；`8d1b3306d09a27b9d8fc29694791d8421aba5f93` 即唯一基线锚点，避免无必要改 refs。
- 本子任务执行时默认不 commit、不 push；完成可审查 diff 后再由用户单独授权提交。
