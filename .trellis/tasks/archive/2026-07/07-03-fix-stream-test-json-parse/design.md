# 技术设计 — 修复流式链路测试前端强解 JSON(P1-4)

## 边界

仅改前端两个文件,后端不动:
- `src/lib/api.ts`:新增 `testsApi.runStream(data)` 函数(POST + `ReadableStream.getReader()` 读 SSE 文本),不修改 `request()`(45-58)与 `testsApi.run`(423-427)。
- `src/pages/RoutesPage.tsx`:`TestPanel`(151-277)按 `stream` 分支:`stream=true` 调 `runStream` 并管理增量文本/取消状态;`stream=false` 维持现有 `testsApi.run` + `testMutation` 路径。新增流式专用状态(`streamedText`、`streamMeta`、`abortRef`、`streamStatus`)。

不动:`src-tauri/src/http/api/tests.rs`、其它 `api.ts` 端点、`request()`。

## 当前数据流(损坏路径)

```
TestPanel[stream=true, 默认]
  → testMutation.mutationFn() → testsApi.run({...stream:true})
  → request<TestResponse>('/tests', {POST, body})            (api.ts:423-427)
  → fetch(...) → resp.ok=true(SSE 200)
  → return resp.json() as Promise<TestResponse>              (api.ts:57) ← 根因
  → SyntaxError(上游是 text/event-stream,非 JSON)
  → testMutation.onError(e)
  → setResult({status:0, body:{}, duration_ms:0, endpoint_id:null, error:e.message})
  → 结果区显示「错误:Unexpected token...」、耗时 0ms、x-test-duration-ms 丢弃
```

`stream=false` 路径正常(后端返 JSON 包装):

```
TestPanel[stream=false] → testsApi.run → request() → resp.json() → TestResponse{status,body,duration_ms,endpoint_id,error}
  → onSuccess(data) → setResult(data) → 结果区 JSON.stringify(result.body) 正常渲染
```

## 修复设计

### 选型:`fetch` + `ReadableStream.getReader()`(非 EventSource)

spec(`app-stack-conventions.md:574`)写「流式用 EventSource」,但 `/api/tests` 是 **POST**。标准 `EventSource` 构造器(`new EventSource(url)`)只支持 GET、不接受 method/body/headers。因此**不能用 EventSource**。唯一可行方案:`fetch(url, {method:'POST', body, signal})` + `resp.body.getReader()` + `TextDecoder` 逐块解码文本。`AbortController` 提供取消。此选型以代码事实(后端 POST 契约)为准,完成后回写 spec 纠正「EventSource」表述(见 implement.md 步骤 3.3)。

### api.ts 新增 `runStream`

在 `testsApi` 对象内新增 `runStream`(与 `run` 并列),返回一个**可取消的流式读取句柄**而非 Promise<TestResponse>:

```ts
export interface TestStreamHandle {
  /** 已读取并解码的文本块回调(每块触发一次,UI 累加)。 */
  onChunk: (text: string) => void;
  /** 流正常结束后的元数据(状态/耗时/端点),由响应头提取。 */
  onDone: (meta: { status: number; duration_ms: number; endpoint_id: string | null }) => void;
  /** 网络/解码错误(含 abort 之外的)。 */
  onError: (err: Error) => void;
  /** 中止读取(用户点停止)。 */
  abort: () => void;
}

runStream: (data: TestRequest, handle: TestStreamHandle): void => {
  const ac = new AbortController();
  (async () => {
    let resp: Response;
    try {
      resp = await fetch(`${API_BASE}/tests`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
        signal: ac.signal,
      });
    } catch (e) {
      // abort 视为取消非错误,单独处理
      if (ac.signal.aborted) return;
      handle.onError(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    const duration = Number(resp.headers.get('x-test-duration-ms') ?? 0) || 0;
    const endpoint_id = resp.headers.get('x-endpoint-id');
    if (!resp.ok || !resp.body) {
      // 上游非 2xx 或无可读流:读 body 文本作错误
      const text = await resp.text().catch(() => '');
      handle.onError(new Error(`${resp.status}: ${text || resp.statusText}`));
      handle.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
      return;
    }
    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        if (value) handle.onChunk(decoder.decode(value, { stream: true }));
      }
      handle.onChunk(decoder.decode()); // flush
      handle.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
    } catch (e) {
      if (ac.signal.aborted) {
        // 已取消:仍回传已读 meta,UI 标记取消
        handle.onDone({ status: resp.status, duration_ms: duration, endpoint_id });
        return;
      }
      handle.onError(e instanceof Error ? e : new Error(String(e)));
    }
  })();
  // 把 abort 暴露给 handle(闭包):见下 TestPanel 用 ref 持有 ac
}
```

> 实现细节(`abort` 暴露方式)可在编码时微调:更简洁的形态是 `runStream` 直接返回 `{ abort: () => ac.abort() }` 而非内置 `abort` 字段——由 implement 阶段定稿,本设计不锁死。核心契约:POST + signal + getReader + onChunk/onDone/onError 三回调。

### TestPanel 分支改造

`stream=true`(默认分支,replace 现有 `testMutation` 单路径):

新增本地状态:
- `streamedText: string` — 累加的 SSE 文本。
- `streamMeta: { status: number; duration_ms: number; endpoint_id: string | null } | null` — 流结束后填统计栏。
- `streamState: 'idle' | 'streaming' | 'done' | 'cancelled' | 'error'`。
- `streamError: string | null`。
- `abortRef: React.MutableRefObject<(() => void) | null>` — 持有当前句柄的 abort。

「发送测试」按钮 `onClick`(`stream` true 时):
```ts
setStreamedText(''); setStreamError(null); setStreamMeta(null); setStreamState('streaming');
const handle = testsApi.runStream({route, path, model, prompt, stream:true}, {
  onChunk: (t) => setStreamedText((prev) => prev + t),
  onDone: (m) => { setStreamMeta(m); setStreamState((s) => s === 'streaming' ? 'done' : s); },
  onError: (e) => { setStreamError(e.message); setStreamState('error'); },
});
abortRef.current = handle.abort;
```

新增「停止」按钮:仅在 `streamState === 'streaming'` 显示,`onClick` 调 `abortRef.current?.()` 并 `setStreamState('cancelled')`。

结果区渲染(`stream` true 时):统计栏用 `streamMeta.status`/`duration_ms`/`endpoint_id`;错误带用 `streamError`;响应体区改为 `<pre>` 显示 `streamedText`(或「已取消」标记),不再 `JSON.stringify(result.body)`。流式进行中可附加「接收中...」提示。

`stream=false`(保持不变):继续用 `testsApi.run` + `testMutation` + `result: TestResponse` + 现有 `JSON.stringify(result.body)` 渲染与统计栏。

> 分支判定:`stream` 是 TestPanel 既有状态(155)。渲染/按钮/状态三处按 `stream` 选择两套,避免互相污染。`result`(TestResponse)仅在 `stream=false` 使用;流式三状态仅在 `stream=true` 使用。可统一一个 `panelMode` 派生,但对最小改动而言直接以 `stream` 三元分支即可。

### headers 透传确认

后端 `tests.rs:96-101` `stream=true` 用 `upstream_resp.into_parts()` 保留上游所有响应头再插入 `x-test-duration-ms`,故前端 `resp.headers.get('x-test-duration-ms')` 可读;`x-endpoint-id` 由 proxy 在上游响应设置(`tests.rs:121` non-stream 分支读取同一头,stream 分支同样透传),前端可读。若上游未设 `x-endpoint-id`(如错误态),`endpoint_id` 为 null,端点栏不显示(与现有 `result.endpoint_id &&` 守卫一致)。

## 兼容性

- `request()`(api.ts:45-58)**不改**:所有 JSON 端点(accounts/endpoints/logs/routes/...)继续复用,行为不变。
- `testsApi.run`(api.ts:423-427)**不改**:`stream=false` 路径与 `TestResponse` 接口不变。
- 后端 `tests.rs` 契约**不改**:`stream=true` 透传 SSE + `x-test-duration-ms`、`stream=false` JSON 包装,本设计完全按既有契约消费。
- RoutesPage 其它部分(RouteCard 设置、候选端点、编辑面板)**不动**;仅 TestPanel 内部行为分支。
- 不引入新依赖:用浏览器原生 `fetch` / `ReadableStream` / `TextDecoder` / `AbortController`(React 18 + Vite 现代浏览器目标均已支持)。

## 测试设计

前端无测试框架(`package.json` 无 vitest/jest/playwright,`src/` 无 `*.test.*`/`*.spec.*`)。AC 落地为**手动验证 + 构建门禁**(见 prd.md AC):

手动步骤:
1. `npm run dev` 启动前端 + `cargo run`(或既有 dev 脚本)起 42567。
2. RoutesPage → 任一路由卡片「链路测试」(确认「流式模式」勾选,默认 `useState(true)` → true)。
3. 配有效上游端点(至少一个 claude-code/codex/v1 候选可达)+ prompt,点「发送测试」。
4. 预期:结果区**逐块**出现 `data: {...}` / `event: ...` SSE 文本(非「Unexpected token」错误);完成后统计栏「耗时」>0(`x-test-duration-ms` 生效)。
5. 测试进行时点「停止」→ 文本停止增长、统计栏标「已取消」、无报错弹窗。
6. 取消「流式模式」勾选,点「发送测试」→ 结果区显示 `JSON.stringify` JSON 体、统计栏正常(回归 `stream=false`)。
7. 断开/伪造不可达上游,`stream=true` → 结果区显示 `status` 与错误文本(非 2xx 路径)。
8. 构建:`npx tsc --noEmit` 0 error、`npm run build` 成功。

> 后端 `tests.rs` 已有单测覆盖契约(透传/JSON 包装),本任务不动后端,无需新增后端测试。

## 风险/回滚

- **风险 1:跨协议上游的非 UTF-8 / 半 chunk 边界**。`TextDecoder.decode(value, {stream:true})` 处理多字节跨块;流尾 `decode()` flush。低风险,标准模式。
- **风险 2:`AbortController` 在已 done 的流上 abort 无害**;`abortRef.current` 在流结束后置 null 或按钮 disabled 防 stale 调用。
- **风险 3:双状态(`result` vs 流式三状态)并存导致渲染错乱**。用 `stream` 严格分支,流式分支不引用 `result`/`testMutation`,`stream=false` 分支不引用流式状态。组件可读性可考虑子组件拆分(`StreamResultBar`/`StreamResultBody`),但最小改动可内联。
- **风险 4:大流量流式文本撑爆 DOM**。`<pre>` 加 `max-h-48 overflow-y-auto`(现有样式 268 行已有)即可;不做截断(测试 prompt 量级小)。
- **风险 5:spec 的「EventSource」表述误导后续维护者**。完成时用 `trellis-update-spec` 纠正为 `fetch + ReadableStream`(POST 端点不支持 EventSource)。
- **回滚点**:单 commit、两文件;`git revert` 即可回滚。回滚后 `stream=true` 仍坏(回到现状),`stream=false` 不受影响(未改其路径)。
