# 执行计划 — 修复流式链路测试前端强解 JSON(P1-4)

## 前置

- [x] PRD / design 已定。
- [x] 代码事实已复核(行号实测与报告一致:api.ts:45-58 / api.ts:423-427 / RoutesPage.tsx:155 / tests.rs:94-102)。
- [x] 选型已定:`fetch` + `ReadableStream.getReader()`(POST 端点不支持 EventSource,spec「EventSource」表述须纠正)。
- [x] 前端无测试框架已确认(`package.json` 无 vitest/jest/playwright,`src/` 无测试文件)→ AC 为手动 + 构建。

## 执行步骤

1. **配置上下文(1.3)** — 填 `implement.jsonl` / `check.jsonl`(spec + 研究文档,不含代码路径):
   - `implement.jsonl`:
     - `{"file": ".trellis/spec/guides/app-stack-conventions.md", "reason": "POST /api/tests 契约(567-574):stream=true 透传 SSE+x-test-duration-ms、stream=false JSON 包装;spec 写「流式用 EventSource」但端点是 POST,本任务改用 fetch+ReadableStream 并在 3.3 回写纠正"}`
     - `{"file": ".trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md", "reason": "P1-4 详情(§3):request() 无条件 resp.json() 对 SSE 强解 JSON,stream=true 测试恒报 JSON 解析失败、x-test-duration-ms 丢弃"}`
   - `check.jsonl`:
     - `{"file": ".trellis/spec/guides/app-stack-conventions.md", "reason": "验收对照:POST /api/tests 契约 + 前端流式读取约定"}`
   - 删除两文件首行 `_example` 占位;`task.py validate` 通过。

2. **激活任务(1.4)** — review gate 后 `task.py start 07-03-fix-stream-test-json-parse`。

3. **实现(2.1)** — 派 `trellis-implement` sub-agent(或 inline),按 design.md 落地:

   3.1 `src/lib/api.ts`:
   - 新增 `export interface TestStreamHandle`(回调字段)+ `runStream: (data, handle) => { abort }`(POST + `signal` + `resp.body.getReader()` + `TextDecoder` 逐块 `onChunk`;流尾读 `x-test-duration-ms` / `x-endpoint-id` 头调 `onDone`;非 2xx / 无 body 调 `onError` + `onDone`;abort 视为取消非错误)。
   - **不修改** `request()`(45-58)与 `testsApi.run`(423-427)与 `TestResponse`(415-421)。
   - 在 `testsApi` 对象(423)内新增 `runStream` 字段,与 `run` 并列。

   3.2 `src/pages/RoutesPage.tsx` `TestPanel`(151-277):
   - 保留既有 `stream`/`path`/`model`/`prompt` 状态与「流式模式」勾选(209-218)。
   - 新增流式专用状态:`streamedText`、`streamMeta`、`streamState`('idle'|'streaming'|'done'|'cancelled'|'error')、`streamError`、`abortRef`(`useRef`)。
   - 「发送测试」按钮 `onClick`:按 `stream` 分支——`true` 调 `testsApi.runStream(...)` 并置 `streamState='streaming'`、reset 状态、`abortRef.current = handle.abort`;`false` 走现有 `testMutation.mutate()`(158-171 不动)。
   - 新增「停止」按钮:仅 `streamState==='streaming'` 显示,`onClick` 调 `abortRef.current?.()` + `setStreamState('cancelled')`。
   - 结果区(240-273)按 `stream` 分支渲染:
     - `stream===true`:统计栏用 `streamMeta`(status/duration_ms/endpoint_id),进行中显示「接收中...」,响应体区 `<pre>` 显示 `streamedText`(取消时附「(已取消)」尾标),错误带用 `streamError`。
     - `stream===false`:维持现有 `result`/`TestResponse` + `JSON.stringify(result.body)` 路径(无改动)。
   - `disabled` 守卫:发送按钮在 `streamState==='streaming'` 或 `testMutation.isPending` 时 disabled。
   - 类型:新函数/状态有明确类型,无 `any`(回调用 `unknown`/具体接口)。

4. **质量检查(2.2)** — 派 `trellis-check`:
   ```bash
   npx tsc --noEmit
   npm run build
   # 注:前端无测试框架,不跑前端单测
   # 后端未改动,可选 cargo check 确认无回归(非必需,因未触 src-tauri)
   ```
   - 全绿:`tsc --noEmit` 0 error、`npm run build` 成功、无新 lint 警告。
   - 重读 `src/lib/api.ts` 确认 `request()` 未被改动。

5. **手动验证(2.2 续)** — 在真实上游可达环境:
   - `stream=true`(默认):逐块增量显示 SSE 文本,耗时 >0,「停止」可取消且不报错。
   - `stream=false`:`JSON.stringify(result.body)` 正常、统计栏正常(回归)。
   - `stream=true` 上游不可达/非 2xx:结果区显示错误态。
   - 步骤详见 design.md「测试设计」与 prd.md AC。

6. **Spec 更新(3.3)** — 用 `trellis-update-spec` 回写 `.trellis/spec/guides/app-stack-conventions.md`「POST /api/tests 契约」节(第 574 行前端说明):
   - 把「流式用 EventSource,`AbortController` 取消」纠正为「流式用 `fetch` + `ReadableStream.getReader()` + `TextDecoder` 逐块读取 + `AbortController` 取消(`/api/tests` 为 POST 端点,标准 `EventSource` 仅支持 GET 故不可用)」。
   - 记录此为前端流式测试读取模式的既定约定,供后续维护者参照。

7. **提交(3.4)** — 单 commit:`fix(ui): stream test panel reads SSE via fetch+ReadableStream instead of forcing JSON parse (P1-4)`
   - scope=`ui`(前端缺陷);仅 `src/lib/api.ts` + `src/pages/RoutesPage.tsx` 两文件(spec 文档变更若走单独 commit 或并入均可,默认并入)。

## 验证命令

```bash
npx tsc --noEmit        # 0 error
npm run build           # 成功(无前端单测可跑)
# 手动:npm run dev + 起后端 42567,按 design「测试设计」逐项验证
```
基线:前端既无测试也无 lint 脚本约束,门禁仅 `tsc` + `build`;前端无单测增量(框架缺失,见 prd.md Notes)。

## 回滚点

- 两文件单 commit;`git revert <commit>` 即回滚。
- 回滚后:`stream=true` 回到现状(JSON 解析失败),`stream=false` 不受影响(未改其路径)。
- 若手动验证发现 `getReader()` 在某浏览器/某上游下行为异常,可在 Stream 状态机加 `stream:true` 的 `TextDecoder` 错误兜底,但优先按 design 标准模式落地。

## 风险文件

- `src/lib/api.ts:45-58`(`request()` — 须确认**未被修改**)。
- `src/lib/api.ts:423-428`(`testsApi` — 新增 `runStream` 字段,不改 `run`)。
- `src/pages/RoutesPage.tsx:151-277`(`TestPanel` — 分支改造主战场,含 155 `useState(true)`、158-171 `testMutation`、240-273 结果区)。
- `.trellis/spec/guides/app-stack-conventions.md:574`(spec「EventSource」表述 — 3.3 步骤纠正)。
