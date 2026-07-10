# 设计 — 真实链路测试与调试器

> 配套 prd.md。本文件只写技术设计：测试 API 契约、RouteProxy test_only 模式、前端 SSE 调试器、Images/Audio 临时展示。

## 1. RouteProxy test_only 扩展

现有 proxy_request 方法增加可选参数：

```rust
pub async fn proxy_request(
    &self,
    route_id: &str,
    req: Request<Body>,
    test_only: bool,  // 新增
) -> Result<Response<Body>, (StatusCode, String)>
```

test_only=true 时：
- **FailoverState**：设 failover_enabled=false（不写 `endpoints.cooldown_until`、不写 `last_failure_at`、不写 `last_success_at`）。
- **Selector**：不过滤冷却中的端点。
- **Logger**：写 `tool='test'`（覆盖 route_id 传入的 tool）。
- 其余完全一致（selector、auth_injector、model_mapper、translator、forward）。

## 2. API 契约

### POST /api/tests

请求：
```json
{
    "route": "claude-code",      // claude-code | codex | v1
    "path": "/v1/messages",       // 转发路径
    "model": "claude-sonnet-4",   // 可选，不提供则用默认
    "prompt": "Hello!",           // 必填，测试消息
    "stream": true                // true=SSE, false=JSON
}
```

处理流程：
1. handler 构建完整请求体：若 route=claude-code 用 Anthropic 格式；codex 用 responses 格式；v1 用 chat 格式。
2. 调用 RouteProxy::proxy_request(route, req, test_only=true)。
3. 返回上游响应（流式或 JSON）。

## 3. 前端 SSE 调试器

- RoutesPage 每条路由卡片底部加「测试」可折叠面板。
- 配置区：path 输入、model 下拉（从 /v1/models 动态加载）、prompt textarea、stream 开关。
- 结果区（流式）：EventSource 接收 SSE → 逐 chunk 追加文本到只读显示区。
- 统计栏：首 token 时间、chunk 数、总耗时（从 SSE event metadata 或页面计时）。
- fallback 链路栏：从日志或响应头解析 hop 列表显示。
- 取消：`AbortController` 中断 EventSource。

## 4. Images/Audio 临时展示

- 测试结果为二进制媒体时，前端用 `URL.createObjectURL(blob)` 创建临时 URL。
- img 标签 / audio 标签展示，附元数据（media_type, content_length, latency）。
- 用户可「另存为」通过浏览器下载。
- 页面刷新或离开后 blob URL 自动释放（不持久化）。

## 5. 测试日志

- 写入 request_logs 表，tool='test'。
- LogsPage 增加过滤 drop-down：全部/生产/测试。

## 6. 修改文件清单

| 文件 | 修改内容 |
|------|---------|
| http/proxy/mod.rs | proxy_request 增加 test_only 参数，test_only 时禁冷却/故障转移状态更新 |
| http/proxy/selector.rs | 新增 skip_cooldown_check 模式（test_only 不跳过冷却端点） |
| http/proxy/failover.rs | 新增 test_only 模式：record_failure 不设置 cooldown |
| http/proxy/logger.rs | 写入时 tool 字段覆盖为 'test'（若 test_only） |
| http/api/tests.rs（新增） | POST /api/tests handler |
| http/api/mod.rs | 注册 tests 模块 |
| http/router.rs | 挂载 /api/tests |
| lib/api.ts | 新增 testsApi 函数和 TestRequest/TestResult 类型 |
| pages/RoutesPage.tsx | 每条路由卡片加测试面板（配置区+结果区+统计+fallback 链） |
| pages/LogsPage.tsx | 加过滤开关（全部/生产/测试） |
