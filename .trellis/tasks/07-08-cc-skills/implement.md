# Implement: Skills 管理（完整 ccs 范围）

## Checklist

1. 建立 `skills` / `skill_repos` 迁移、DAO 与数据类型，补 CRUD 单测。
2. 实现 SSOT 管理：默认 app data skills、可选 `~/.agents/skills`、目录结构校验、content hash、路径安全 helper。
3. 实现本地目录/zip 导入与卸载备份；保证无效 skill、越界路径、恶意 symlink 被拒绝。
4. 实现多应用目标路径映射与 sync engine：auto/symlink/copy、目标缺失 no-op、非托管同名 conflict、全量 sync。
5. 实现 REST API：list/get/import-dir/import-zip/enable/disable/sync/status/scan-unmanaged/backups/restore。
6. 实现 `/skills` 前端 MVP：已安装列表、导入、各 app 开关、同步状态、冲突提示、备份恢复。
7. 实现 GitHub/skills.sh discovery：search/install/check-updates/update/batch-update，所有网络动作用户显式触发。
8. 补完整前端与后端测试，并执行跨平台路径回归。

## Current Implementation Notes

- 已完成本地安全地基：`skills` / `skill_repos` 迁移与 DAO、app data SSOT、本地目录导入、`SKILL.md` 校验、内容 hash、拒绝符号链接、copy 投影、托管标记、非托管同名冲突保护、多 app 启用/禁用/sync/status、`/skills` 页面 MVP。
- 当前投影策略固定为 copy；`auto`/`symlink` 暂未实现，API/UI 会显示 copy 投影提示。
- `import-zip`、`install-repo`、`scan-unmanaged`、`backups`、`restore`、`search`、`check-updates`、`update` 已挂 REST 路由但返回明确 501，等待后续阶段补齐，不自动联网。
- 当前不实现卸载/备份恢复与网络发现/更新，因此 AC6/AC7/AC8/完整 AC10 仍未满足；本阶段覆盖 AC1/AC2/AC3/AC4/AC5/AC9/AC11 的本地安全子集。

- `cargo test`
- `npm test -- --run`
- `npm run build`

## Review Gates

- 路径安全 review：所有删除/copy/symlink 目标必须在 SSOT 或允许 live root 内。
- conflict review：非托管同名目录不得被默认覆盖或删除。
- 网络 review：外部安装/更新必须显式触发并展示来源预览。
- 边界 review：grep 确认不写 `settings.json`、`~/.claude.json`、`CLAUDE.md`、`projects/`。

## Rollback Points

- 阶段 A 完成后即可单独验收本地导入 + sync；若网络发现复杂度过高，可暂停在阶段 B 并保留 PRD 完整范围。
- sync engine 上线前保留 dry-run/status 输出，确认路径后再开放删除/替换动作。
- 任何更新/卸载失败都必须能从备份恢复 SSOT 与 live 投影。