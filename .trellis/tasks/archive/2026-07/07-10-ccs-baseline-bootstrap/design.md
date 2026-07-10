# Design：ccs 基线 bootstrap

## 目标

只建立可靠地基：保护旧主树 → 当前目录切换到 ccs release 根 → 原样验证 → 迁入 Trellis → 基于 ccs 刷新规范。任何产品删改均在后续子任务。

## Git/文件流

```text
main + untracked Trellis
  ├─ copy untracked → repo 外 bootstrap backup + manifest/hash
  ├─ git switch -c agent-switch-ccs 8d1b3306...
  ├─ validate untouched ccs
  ├─ restore tracked Trellis from main
  ├─ restore untracked task/audit/archive from backup
  ├─ merge .gitignore
  └─ regenerate ignored platform adapters + refresh specs
```

不使用 stash 作为唯一保护：stash 默认不含 untracked，`-u` 也不如显式外部备份可审计。禁止 reset/clean。

## 基线验证分层

1. 来源/版本/干净度。
2. 精确工具链与 frozen install。
3. 前端官方 CI 门。
4. Rust 官方 CI 门 + locked/check 补充。
5. `tauri build --no-bundle` release executable。
6. 完整 installer/updater artifact（凭据可用时）。
7. 隔离环境启动（避免真实 `~/.cc-switch`）。

每层独立记录“通过/失败/阻塞”，上层阻塞不抹去下层可验证结果。

## Trellis 迁入边界

- 恢复：`.trellis` workflow/scripts/agents/config、任务/archive/journal、必要 audit。
- 合并：根 `.gitignore`。
- 再生成：`.claude/` 等 ignored 平台适配。
- 禁止恢复：旧 product source/manifests/release docs。
- 旧 product specs 不直接生效：先归档 legacy，再 bootstrap ccs-backed specs。

## 提交边界

- base：`8d1b3306…`（纯 ccs，不新增 commit）。
- bootstrap commit：Trellis/task/ignore only。
- spec commit：ccs-based specifications。

若用户未授权 commit，两个逻辑 diff 必须保持可分离，停止在 review gate。

## 风险/回滚

- 外部备份失败 → 不切分支。
- checkout 冲突 → 停止，不删除文件。
- 基线测试失败 → 判断环境/上游，不修产品。
- Trellis 迁错产品文件 → 立即 restore 到纯基线，重新按 allowlist 迁入。
- 随时 `git switch main` 回旧产品；外部备份保留至用户确认删除。
