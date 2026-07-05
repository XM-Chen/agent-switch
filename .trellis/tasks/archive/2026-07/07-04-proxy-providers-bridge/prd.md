# 代理模式与 providers 桥接及升级回填

## Goal

P1 阶段不重写 selector 管道。proxy 模式 provider 的语义 = 工具指向本地代理(`tool_takeover` 写本地代理 URL + 占位符),上游仍由现有 `endpoints` 管道按 model/capability/priority 路由。本任务把子任务 1/2/3 的成果接成可用闭环,并保证从 v7 之前(无 providers 表)升级到 v8 后存量用户无缝:已启用接管的 claude-code/codex 各回填一个 `mode=proxy` 默认 provider(`is_current=1`)。

## Background(已确认事实)

- `http/proxy/selector.rs` 从 `endpoints::list_enabled` 加载候选,完全不引用 `tool_takeover`/`providers`/`is_current`。proxy provider 与上游选路解耦——印证"不重写 selector"。
- `tool_takeover` 表迁移 v4 创建,无 seed;运行时由旧 API 写入 `enabled=1` 表示接管中。v8 加了 `mode`/`active_provider_id` 列,默认 `mode='proxy'`。
- `providers` 表 v7 创建,无 seed;`is_current` 由 partial unique index `idx_providers_current` 保证每 app_type 至多一个。
- 子任务 2 已实现 `tool_takeover::enable(db, tool, data_dir)`(proxy 模式走 `apply()` 写本地代理 + 占位符)、`enable_direct`、`disable` direct→proxy 回退。
- 子任务 3 已实现 `POST /api/providers/{id}/switch`:set_current + 按 mode 调 enable/enable_direct + 失败回滚。
- 现有 portability(collect/apply)只导出 `tool_takeover {tool, enabled}`,不导出 providers。升级回填不依赖 portability。

## Requirements

### R1 升级回填(迁移后一次性)
- 在 v8 迁移完成后(或 app 启动首次见到 `providers` 为空且 `tool_takeover` 有 `enabled=1` 行时),为每个 `tool_takeover WHERE enabled=1` 的 tool(claude-code/codex)生成一个默认 provider:
  - `id`: 确定性(如 `prov-backfill-<tool>`),保证幂等可重入。
  - `app_type`: tool→app_type 映射(claude-code→`claude-code`,codex→`codex`)。
  - `name`: 本地化默认名(如"默认代理(claude-code)")。
  - `mode='proxy'`,`settings_config='{}'`(proxy provider 不需要 endpoint 引用,上游由 endpoints 管道决定)。
  - `is_current=1`,`sort_index=0`。
- 幂等:已存在同 id(或同 app_type 已有 current)时不重复创建/不覆盖。

### R2 一致性边界(已定)
- 只在回填时保证 `tool_takeover` 与 `providers.is_current` 一致(回填前提即 `tool_takeover.enabled=1`,二者天然对齐)。
- **运行期不做主动一致性校验或自动修复**。子任务 3 的 switch 已走 set_current + enable 原子化 + 失败回滚,正规路径不脱节;启动期静默改 DB 反而难追。脱节边缘情况(旧 API/手动操作)留给 P1 后深度绑定阶段统一处理。

### R3 集成测试
- 回填后转发行为与改造前一致:`tool_takeover.enabled=1` + `providers.is_current=1(proxy)` → 请求经本地代理 → selector 从 `endpoints` 选路上游。
- 回填幂等:重复执行不产生重复 provider。
- 回填前已有 current provider 时不覆盖。

### R4 约束
- 不重写 selector / failover / translate 管道。
- 不动前端(子任务 5)。
- providers→endpoints 深度绑定(如 provider 内嵌 endpoint_id 用于 proxy 选路过滤)留待 P1 后独立阶段。

## Acceptance Criteria

- [ ] 从空 providers + `tool_takeover.enabled=1`(claude-code 和/或 codex)升级,回填后每个启用 tool 各有一个 `mode=proxy`、`is_current=1` 的 provider。
- [ ] 回填幂等:二次启动不重复创建、不覆盖用户改动。
- [ ] 回填不覆盖已存在的 current provider。
- [ ] 回填后经本地代理的转发行为与改造前一致(集成测试)。
- [ ] 全量门禁:fmt / clippy -D warnings / cargo test 全绿。

## Out of Scope

- providers→endpoints 深度绑定(P1 后独立阶段)。
- selector 管道重写。
- 前端切换器页面(子任务 5)。
- 运行期 `tool_takeover` ↔ `providers.is_current` 一致性校验/自动修复(见 R2)。
