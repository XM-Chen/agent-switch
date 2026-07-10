# 审计执行计划

## 执行顺序

1. **P0 现状摸底（只读）**
   - 读 `package.json` / `Cargo.toml` / `tauri.conf.json`，确认技术栈与服务地址。
   - `git log --oneline`，对照归档任务确认交付脉络。
   - 列 `src-tauri/src` 与 `src` 目录树，定位模块。

2. **P1 逐域静态审计（只读）**
   - 按 `design.md` 的 D0–D9 域，逐域 grep/read 关键文件，判定实现是否存在、是否达标。
   - 每域记录：证据文件路径、命中/缺失、与 PRD 验收条对照。

3. **P2 非破坏性质量门**
   - `tsc --noEmit`（前端类型检查）
   - `cargo check`（在 `src-tauri`）
   - `cargo fmt --check`
   - `cargo clippy --all-targets`（若可控）
   - 记录每条命令的退出码与关键输出；失败如实记录。`npm run build` 若因环境依赖缺失失败，单独标注为环境问题而非代码问题，并用 `tsc --noEmit` 替代前端正确性验证。

4. **P3 历史会话澄清**
   - `trellis mem search` / `trellis mem extract` 补充用户原始口径。
   - 标注 OpenCode 平台不可索引的限制。

5. **P4 汇总与报告**
   - 产出中文审计报告：决策级摘要 + 需求域矩阵 + 超额完成清单 + 无法验证清单 + 下一步建议。
   - 报告在对话中给出，并把结论指针沉淀到任务。

## 验证命令

```bash
npx tsc --noEmit
cargo check --manifest-path src-tauri/Cargo.toml
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --all-targets --manifest-path src-tauri/Cargo.toml
```

## 回滚点

- 本任务不修改业务代码，无需代码回滚。
- 若某验证命令对环境有副作用（如写入缓存），不影响业务状态，可接受。

## 风险与边界

- 不调用真实上游、不写真实工具配置、不改业务代码。
- OpenCode 历史不可索引 → 相关历史口径只能从 Claude/Codex 会话与任务文档间接确认。
- 静态审计无法证明端到端运行时正确性，只能证明"实现存在 + 可构建/可检查通过"；运行时结论标注为"无法验证"或"需人工运行"。

## 已识别的下一步工作（不属于本审计任务，留作独立任务规划依据）

审计在 D5/D6（路由与故障转移核心）发现 `src-tauri/src/http/proxy/mod.rs` 主转发循环存在实现缺口，各子模块（translator、StreamGuard、model_locks DAO、failover 状态机、oauth_refresh）均已实现但未在主循环正确接入。具体缺陷清单与修复方案见独立任务规划，本审计任务不展开实现。
