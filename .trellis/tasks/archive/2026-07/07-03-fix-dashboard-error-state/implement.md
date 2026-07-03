# 执行计划 — 修复 Dashboard 无 error 态误触发引导(P1-5)

## 前置

- [x] PRD / design 已定。
- [x] 代码事实已逐行复核(7 个 useQuery 行锚点 + CountCard + SectionCard + EmptyGuide 守卫全部确认;与审计报告无锚点出入)。

## 执行步骤

1. **配置上下文(1.3)** — `implement.jsonl` / `check.jsonl` 加真实 spec/research 条目(删 seed `_example` 行):
   - `implement.jsonl`:
     - `{"file": ".trellis/spec/guides/app-stack-conventions.md", "reason": "前端 API 客户端约定节:queryKey 命名、error 抛 fetch 契约(DashboardPage 消费侧补 error 解构须遵守)"}`。
     - `{"file": ".trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md", "reason": "P1-5 详情:7 useQuery 漏 error、CountCard 仅判 loading、EmptyGuide 守卫缺 !anyError"}`。
   - `check.jsonl`:同 `app-stack-conventions.md` 条目(检查 per-widget error 渲染约定符合性)。
   - `python .trellis/scripts/task.py validate` 通过。

2. **激活任务(1.4)** — review gate 后 `python .trellis/scripts/task.py start 07-03-fix-dashboard-error-state`。

3. **实现(2.1)** — 派 `trellis-implement` sub-agent(或 inline),改 `src/pages/DashboardPage.tsx`:
   - **7 个 useQuery 补 error 解构**(`43-70`):每行解构加 `error: <资源>Error`(accountsError/endpointsError/modelsError/routesError/toolsError/logsError/autoRefreshError)。
   - **anyError 聚合**(在 `:96` allLoaded 后):
     ```tsx
     const anyError = !!(accountsError || endpointsError || modelsError ||
       routesError || toolsError || logsError || autoRefreshError);
     ```
   - **EmptyGuide 守卫**(`:108`):`{!anyError && allLoaded && totalResources === 0 && (<EmptyGuide navigate={navigate} />)}`。
   - **CountCard**(`277-300`):props 加 `error?: unknown`;渲染分支 `loading → 骨架 | error → <p text-red-500 text-sm font-semibold title={msg}>加载失败</p> | value`;4 个 CountCard 调用(`114/121/128/135`)传 `error={accountsError}`/`error={endpointsError}`/`error={modelsError}`/`error={routesError}`。
   - **SectionCard**(`304-340`):props 加 `error?: unknown`;渲染分支 `loading → 骨架 | error → <p text-sm text-red-500>加载失败: {msg}</p> | children`;4 个 SectionCard 调用(`146/163/209/247`)传 `error={toolsError}`/`error={autoRefreshError}`/`error={endpointsError || routesError}`/`error={logsError}`。
   - **不改**:`main.tsx`(retry:1)、`api.ts`(throws)、`AppShell.tsx`、其它页面、`aggregateEndpointHealth`/派生计数逻辑、`logs = logsResp?.items ?? []`。

4. **质量检查(2.2)** — 派 `trellis-check`:
   ```bash
   npx tsc --noEmit
   npm run build
   ```
   静态门全绿(tsc 0 error + build 成功)。前端无 test runner,无单测。后端无改动,`cargo check`/`cargo test` 无需跑(若有疑虑可旁证 `cargo check` 仍 0 error,但本任务 0 Rust 改动)。

5. **手动验证(2.2 / 3 验收)** — 桌面 GUI 环境 `npm run tauri dev`:
   - 场景 A:kill 后端 42567(或断网)→ 刷新 `/` → 4 CountCard 显示「加载失败」、4 SectionCard 显示「加载失败: {msg}」、**无 EmptyGuide**(AC1-AC3)。
   - 场景 B:全新数据库空资源 → 刷新 `/` → EmptyGuide 正常渲染(AC4)。
   - 场景 C:有资源 → 刷新 `/` → 全 widget 真实数据,无错误指征,无 EmptyGuide(AC5)。
   - 无 GUI 环境(WSL)则跳过手动,留桌面环境最终实测,代码层 AC1-AC6 由静态门 + 代码 review 保证。

6. **Spec 更新(3.3)** — 用 `trellis-update-spec` 在 `app-stack-conventions.md`「前端 API 客户端约定」节追加一条「Dashboard / 多 query 聚合页面 error 态约定」:
   - 内容:聚合多 useQuery 的概览页必须每 query 解构 `error`,聚合 `anyError`,首空引导守卫须含 `!anyError` 与 `allLoaded && totalResources === 0` 并列;widget 内联渲染 per-widget「加载失败: {error.message}」沿用既有约定,不引 banner。
   - 理由:本 P1-5 暴露的「聚合页面漏 error 解构 → 错误态回退空数组 → 误触首空引导」是可复现的反模式,固化到 spec 防后续其它聚合页(如未来 `/api/dashboard/summary` 消费页)重蹈。

7. **提交(3.4)** — `fix(ui): dashboard error states instead of silent empty fallback (P1-5)`。

## 验证命令

```bash
npx tsc --noEmit     # 必须 0 error
npm run build        # tsc --noEmit && vite build 成功
# (前端无 test runner,手动 GUI 验证见步骤 5)
```

## 回滚点

- 单文件 `src/pages/DashboardPage.tsx` + 两局部子组件(CountCard/SectionCard)签名扩展;无导出、无跨文件影响。
- `git revert <commit>` 单 commit 回滚。
- 父任务 `07-03-fix-audit-p1-defects` 其余 P1 子任务独立,无依赖耦合。

## 风险文件

- `src/pages/DashboardPage.tsx:43-70`(7 个 useQuery 解构)。
- `src/pages/DashboardPage.tsx:90-108`(anyError 聚合 + EmptyGuide 守卫)。
- `src/pages/DashboardPage.tsx:114-141`(4 个 CountCard 调用处传 error)。
- `src/pages/DashboardPage.tsx:146-269`(4 个 SectionCard 调用处传 error)。
- `src/pages/DashboardPage.tsx:277-300`(CountCard 组件签名 + 渲染分支)。
- `src/pages/DashboardPage.tsx:304-340`(SectionCard 组件签名 + 渲染分支)。

## 前置约定(防误改)

- 既有 per-widget error 渲染模式(`{error && <p className="text-red-500">加载失败: {error.message}</p>}`)在 8 处其它页面一致出现,实现时沿用此文案与样式,不发明新风格。
- EmptyGuide 守卫的 `allLoaded` 现仅含 4 个 query(accounts/endpoints/models/routes)——**不要**因 anyError 聚合了全部 7 个就误把 allLoaded 也改全 7 个;allLoaded 维持「首空引导仅看首空四资源」语义,anyError 维持「任一错即不引导」语义。两条件并列各有职责,改 allLoaded 会改变 empty 触发语义(回归风险)。
- CountCard error 显示选「加载失败」文字而非 `—`,理由:文字明确告知运维失败,`—` 易与「无值」混淆;title 属性携带完整 error.message 供 hover 查看。
