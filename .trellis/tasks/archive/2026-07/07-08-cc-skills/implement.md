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

### 阶段 A/B（本地安全地基，已完成）

- `skills` / `skill_repos` 迁移与 DAO、app data SSOT、本地目录导入、`SKILL.md` 校验、内容 hash、拒绝符号链接、copy 投影、托管标记、非托管同名冲突保护、多 app 启用/禁用/sync/status、`/skills` 页面。
- 投影策略固定为 copy；`auto`/`symlink` 未实现，API/UI 显示 copy 投影提示（PRD R3.3 的 symlink/auto 留作后续增强，不影响 AC）。

### 阶段 C（网络发现/安装/更新/备份，已完成）

- 拆分 `services/skills` 为子模块：`download`（GitHub tarball 下载 + tar/zip 安全解包，拒绝 tar-slip/绝对路径/符号链接/硬链接 + subdir 定位）、`install`（install_repo/import_zip，复用 `land_skill` 落地）、`backup`（uninstall/list_backups/restore，SSOT 副本 + DB 行快照）、`update`（check_updates/update 单批量，三阶段回滚）、`discovery`（GitHub Search + scan_unmanaged 只读扫描）。
- DAO 新增 `update_repo_check` / `find_repo_for`；`land_skill` 抽出为 import-dir/install-repo/import-zip 共用落地路径。
- 8 个原 501 端点全部落地为真实 handler；新增 `DELETE /api/skills/{id}` 卸载。GitHub token 可选（`app_metadata` key `skills_github_token`，匿名首版可用，仅用于提升限速）。
- **更新安全**：网络/hash 失败发生在改写 SSOT 之前，绝不删除健康 skill；改写 SSOT 前强制备份，任一步失败明确从「本次备份」恢复（不误用其它备份）。
- 前端 SkillsPage 全量覆盖：GitHub 安装、zip 导入、搜索发现（含从候选一键安装）、检查更新、批量/单个更新、卸载、备份恢复、未托管扫描；删除/覆盖/网络/批量操作均二次确认；loading/error/empty/conflict 分状态展示。
- **DeepLink 解锁**：`services/deeplink` 的 `import`/`import_skill` 改 async，skill 资源接入 `install_repo`，preview 解除 `blocked`（见 07-08-cc-deeplink）。

### AC 覆盖

- AC1-AC11 全覆盖。AC12 中的跨平台路径回归（Windows symlink 权限、大小写敏感等）留作后续手动验证。
- skills.sh 专用发现契约未核实，首版 search 按 GitHub Search API 实现（PRD R5.1 的 skills.sh 入口作为已知接口后补，不阻塞 AC8 的「搜索并安装候选」）。

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