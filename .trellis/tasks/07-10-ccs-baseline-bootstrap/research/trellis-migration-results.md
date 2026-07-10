# Trellis 迁入与规范刷新验证结果

> 执行日期：2026-07-10。分支：`agent-switch-ccs`。产品基线 HEAD：`8d1b3306d`。

## 迁入边界

### 从 `main` 恢复

- Trellis 0.6.6 runtime：`.version`、`.template-hashes.json`、`scripts/`、`agents/`、`config.yaml`、`workflow.md`；
- 框架自身规范：`.trellis/spec/trellis-runtime/`；
- 48 个已完成旧任务：只放在 `.trellis/tasks/archive/`，没有恢复成 active task；
- workspace journal：保留历史工作记录。

### 从仓库外备份恢复

备份目录：`E:/SynologyDrive/git_files/agent-switch-bootstrap-07-10/`。

- `.trellis-upgrade-audit.json`：SHA-256 `0cf61bf5189ef9ed143401c0166d3b4e16909fe23a1457f0c416b7fd22df6f66`，与 `manifest.txt` 一致；
- `.trellis/archive/`：补回 upgrade audit/snapshot 等 403 个文件；现有 archive 与备份没有文件内容冲突；
- 父/子任务：当前会话版本比 13:01 的备份新，故保留当前版本，不用旧备份覆盖。

备份未删除。

### 明确未迁入

- 旧 Agent Switch 产品源码、package manifest、release 文档；
- 旧 active spec 直接指导当前实现；
- 被忽略的平台 runtime 状态、secret、build artifact。

`git status --short -- src src-tauri package.json pnpm-lock.yaml` 返回空，产品源码保持纯 ccs v3.16.5。

## `.gitignore`

以 ccs 文件为基础追加：

- Trellis 生成平台目录 `.agents/.cursor/.opencode/.pi/.reasonix/.trae/.zcode`；
- updater 私钥 `*.key/*.key.pub` 与生成 `latest.json`；
- 保留 ccs 原有 node/dist/release/Tauri/IDE/Flatpak 忽略项。

`.trellis/.gitignore` 继续忽略 `.developer`、`.current-task`、`.runtime`、`.backup-*`、临时文件。

## Runtime 更新与设备状态

- 项目 `.trellis/.version` = 0.6.6；CLI = 0.6.6；
- `trellis update --skip-all` 自动更新 27 个被识别为旧模板的 runtime 文件，保留项目自定义 `config.yaml` / `.trellis/.gitignore`；
- `session_auto_commit: false`，保证任务/session 生命周期不会绕过用户授权自行提交；
- 重新初始化本地 developer = `xm-chen`；`.developer` 被忽略；
- `.claude/` 平台适配已存在且被根 `.gitignore` 忽略，无需覆盖式 init。

## Active spec refresh

- 旧 `main` 的 frontend/backend/guides 共 18 个文件归档到 `.trellis/spec/legacy-agent-switch-0.2.2/`，并有醒目 README 禁止直接使用；
- 新建 15 个中文 active spec：frontend 4、backend 6、guides 5；
- 规范锁定 ccs v3.16.5 事实与 Agent Switch 目标不变量，重点覆盖：
  - `settings_config` 全文快照唯一 SSOT，不新增 `meta.snapshot`；
  - Common Config 深合并/剥离和切出回填；
  - schema v11 暂留；
  - Copilot/Codex OAuth/OpenRouter/Responses 保护；
  - 非 loopback 全路由鉴权（标注为待实现，未假装 ccs 已具备）；
  - WebDAV/S3 明文 SQL 风险确认；
  - Windows/中文/Claude-only 跨层裁剪；
  - Agent Switch 身份、数据根、Deep Link、WiX/updater；
  - 每类变更的验证矩阵。
- 相对 Markdown 链接检查：0 个坏链接。

## Trellis 功能验证

| 项 | 结果 |
|---|---|
| `task.py list` | ✅ 只显示父任务 + bootstrap 子任务，旧任务均在 archive |
| `task.py current` | ✅ 当前指向 bootstrap |
| `task.py start` | ✅ 可按 session 写入 `.current-task` |
| `task.py create` | ✅ 临时 smoke task 创建成功并清理；HEAD 未变化 |
| context validate | ✅ bootstrap `implement.jsonl` 4 个真实条目，`check.jsonl` 3 个真实条目 |
| phase context | ✅ phase index 与 step 2.1 可加载，dispatch 规则存在 |
| CLI update dry-run | ✅ runtime 无待自动更新；仅自定义 config/.gitignore 需保留 |
| root HEAD | ✅ 仍为 `8d1b3306d chore(release): v3.16.5` |
| 产品源码改动 | ✅ 0 |
| push/release/default branch | ✅ 未执行 |

## 提交边界

1. Trellis bootstrap：runtime、archive/workspace、父子任务、R3/迁入记录、ignore；
2. ccs-based spec refresh：legacy 归档 + 15 个 active spec。

只创建本地提交，不 push。
