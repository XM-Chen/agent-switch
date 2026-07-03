# 修复流式链路测试前端强解 JSON 导致测试整体损坏(P1-4)

> 父任务:`07-03-fix-audit-p1-defects`。审计来源:`codebase-audit` 报告 §3 P1-4。

## Goal

修复 RoutesPage TestPanel 在 `stream=true` 时复用 `testsApi.run` → 共享 `request()` 无条件 `resp.json()` 导致对所有 SSE 响应强解 JSON、抛 SyntaxError、流式测试永远显示「JSON 解析失败」的端到端损坏。使 `stream=true` 走独立的流式读取路径(`fetch` + `ReadableStream.getReader()` 逐块读文本并增量显示、捕获 `x-test-duration-ms` 头),`stream=false` 继续走现有 JSON 路径不变。

## Background(代码事实)

审计报告 P1-4(`src/lib/api.ts:423` / `src/lib/api.ts:45`)描述:RoutesPage TestPanel 默认 `stream=true`,用户点「发送测试」→ `testsApi.run` 调共享 `request<TestResponse>('/tests', {POST})`,`request()` 通过 `resp.ok`(SSE 返回 200)后无条件 `return resp.json()`,后端 `tests.rs` 在 `stream=true` 透传上游 `text/event-stream` 原始响应体(非 JSON)→ `resp.json()` 抛 SyntaxError 进 `onError`,`x-test-duration-ms` 头被丢弃(`duration_ms=0`)。

**代码复核确认报告全部锚点准确**(行号实测一致):
- `src/lib/api.ts:45-58` `request<T>()`:`45` 函数定义,`46-49` `fetch`,`50-53` 非 `resp.ok` 抛错,`54-56` 204 返回 undefined,`57` 无条件 `return resp.json() as Promise<T>`。是 SSE 强解 JSON 的根因。
- `src/lib/api.ts:423-427` `testsApi.run`:`run: (data: TestRequest) => request<TestResponse>('/tests', { method: 'POST', body: JSON.stringify(data) })`,强制走 `request()`。
- `src/pages/RoutesPage.tsx:155` `const [stream, setStream] = useState(true);` — 默认 `stream=true` 与报告一致。
- `src/pages/RoutesPage.tsx:156` `const [result, setResult] = useState<TestResponse | null>(null);`。
- `src/pages/RoutesPage.tsx:158-171` `testMutation`:`mutationFn: () => testsApi.run({...stream...})`,`onSuccess: data => setResult(data)`,`onError: e => setResult({status:0, body:{}, duration_ms:0, endpoint_id:null, error:e.message})`。即 SyntaxError 落入 onError,error 文本显示「Unexpected token...」。
- `src-tauri/src/http/api/tests.rs:94-102` `stream=true` 分支:`let (mut parts, body) = upstream_resp.into_parts(); parts.headers.insert("x-test-duration-ms", ...); Response::from_parts(parts, body)`——直接透传上游 SSE 响应体,仅附加 `x-test-duration-ms` 头,**不**做 JSON 包装。
- `src-tauri/src/http/api/tests.rs:103-138` `stream=false` 分支:缓冲响应体,返回 JSON `{status, body, duration_ms, endpoint_id, error}`,与 `TestResponse` 接口对齐。
- 全仓前端 grep `EventSource|getReader|ReadableStream|TextDecoder|AbortController` 零命中——无任何流式读取基础设施,本任务新增。

> 审计 §1/§6 与报告「去重合并」注记一致:`api.ts:45`(finder-2「frontend-ui」维度)与 `api.ts:423`(finder-1「route wiring」维度)为同源缺陷,合并为 P1-4 单条,不计入 dropped。本子任务一并修复 `request()` 的强解 JSON 行为被复用点带来的影响(仅针对流式测试改路,`request()` 本身对其它 JSON 端点正确,不改)。

## Requirements

1. **stream=true 不再走 `testsApi.run`**(不复用 `request()`):在 TestPanel 内按 `stream` 分支,`stream=true` 时走独立的流式 `fetch` 路径,使用 `ReadableStream.getReader()` + `TextDecoder` 逐块读取 SSE 文本并增量显示。
2. **逐块显示流式输出**:用本地状态累加已读文本块,实时渲染到结果区(而非 `JSON.stringify(result.body)`)。流结束后保留完整文本。
3. **保留 `x-test-duration-ms`**:从 `fetch` 响应头读取 `x-test-duration-ms` 填入 `duration_ms` 统计栏;同时读取透传的 `x-endpoint-id` 填入端点栏(后端流式分支虽未显式插入 `x-endpoint-id`,但 `upstream_resp.into_parts()` 保留上游头,前端可读)。
4. **可取消**:`stream=true` 路径持有 `AbortController`,提供「停止」按钮中止正在进行的流式请求(`signal` 传给 `fetch`,触发后停止读取并标记已取消)。
5. **stream=false 行为不变**:继续走 `testsApi.run` → `request<TestResponse>()` JSON 路径,`TestResponse` 接口、结果区渲染、统计栏均不变。
6. **后端 `tests.rs` 不改**:契约(`stream=true` → 透传 SSE + `x-test-duration-ms`;`stream=false` → JSON 包装)已是正确设计,故障在前端复用层。
7. **不破坏 `request()` 对其它端点**:`src/lib/api.ts:45-58` 的 `request()` 对所有 JSON 端点正确(全仓唯一复用的 HTTP 工具),本任务**不修改 `request()` 签名与默认行为**,仅在 `api.ts`(或新 helper)增加一条仅供 TestPanel 流式分支调用的 `runTestStream()` 函数。
8. **类型与构建**:`tsc --noEmit` 0 error、`npm run build` 成功;不引入 `any`(必要处用 `unknown`/具体类型),新函数有明确返回类型。

## Acceptance Criteria

> 前端无测试框架(已确认 `package.json` 无 vitest/jest/playwright,`src/` 下无 `*.test.*`/`*.spec.*` 文件),故 AC 为**手动验证 + 构建门禁**,不以单测表达。

- [ ] **手动验证 stream=true**:打开 RoutesPage 任一路由卡片「链路测试」面板,确保「流式模式」勾选(默认应已勾选),填写有效上游与 prompt,点「发送测试」→ 结果区**逐块增量显示真实 SSE 文本**(如 `event: ...` / `data: {...}` 行),而非「错误:Unexpected token...」JSON 解析失败态。
- [ ] **手动验证 stream=true 耗时与端点**:stream=true 测试完成后统计栏「耗时」显示真实 `x-test-duration-ms`(>0),非 `0ms`;若上游回 `x-endpoint-id` 则端点栏显示该端点 id。
- [ ] **手动验证 stream=true 可取消**:测试发送中点「停止」→ 读取立即停止,结果区保留已收到片段,统计栏标记「已取消」(不报错)。
- [ ] **手动验证 stream=false**:取消勾选「流式模式」,点「发送测试」→ 结果区显示 `JSON.stringify(result.body)` JSON 体、状态/耗时/端点栏正常(行为与修复前一致,无回归)。
- [ ] **手动验证 stream=true 上游错误**:上游返回非 SSE(如 502/超时)→ 结果区显示错误态(后端透传路径下,fetch 可读 `resp.ok`/状态;非 2xx 按错误处理)。
- [ ] **构建门禁**:`npx tsc --noEmit` 0 error;`npm run build` 成功。
- [ ] `src/lib/api.ts` 的 `request()`(45-58)未被修改,其它 JSON 端点(accounts/endpoints/logs/...)调用不受影响。

## Out of Scope

- **P1-5**(Dashboard 错误态静默误导)——另立任务。
- **P1-1/P1-2/P1-3**(passthrough multipart / ChatToAnthropic content_block_stop / 重复 OAuth)——与本任务无文件重叠,独立任务。
- **后端 `src-tauri/src/http/api/tests.rs`** 不改:流式透传 + JSON 包装契约正确,故障在前端复用层。
- **前端流式通用化**:不抽前端 SSE 通用 hook/工具库,仅修 TestPanel 局部;若后续其它页面需要流式再抽。
- **LogsPage「生产」过滤**(P2-18)、其它 P2/P3——不属于本轮。

## Notes

- 对照 `.trellis/spec/guides/app-stack-conventions.md`「POST /api/tests 契约」节(567-574 行)落地。该节明确规定 `stream=true → 透传上游 text/event-stream,附 x-test-duration-ms 头`、`stream=false → JSON 包装`,并要求「流式用 EventSource,`AbortController` 取消」。
- **spec 与实现的出入**(须在 design.md 标注并据此选型):spec 写「流式用 EventSource」,但 `/api/tests` 是 **POST** 端点,标准 `EventSource` 构造器只支持 GET、不支持自定义 method/body/headers。故**不能**用 `EventSource`,须用 `fetch` + `ReadableStream.getReader()` + `AbortController`。本任务以代码事实(POST 契约)为准选 `fetch+getReader`,并在完成时用 `trellis-update-spec` 将 spec 的「EventSource」纠正为「fetch + ReadableStream(POST 端点不支持 EventSource)」,避免后续误导。
- 前端无测试框架的事实影响 AC 表达:无法写前端单测,AC 落地为手动步骤 + `tsc`/`npm run build` 构建门禁。手动验证须在真实上游可达环境下执行(非流式路径同样依赖真实上游)。
