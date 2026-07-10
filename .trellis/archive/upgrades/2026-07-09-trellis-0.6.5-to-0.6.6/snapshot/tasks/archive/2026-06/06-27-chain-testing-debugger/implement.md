# 执行计划 — 真实链路测试与调试器

> 配套 prd.md / design.md。按依赖顺序推进。

## 实现顺序

### 1. RouteProxy test_only 扩展

- [ ] http/proxy/mod.rs: proxy_request 增 test_only: bool 参数。
- [ ] http/proxy/failover.rs: 增 test_only 模式（record_failure 不更新 cooldown）。
- [ ] http/proxy/selector.rs: 增 skip_cooldown_check。
- [ ] http/proxy/logger.rs: test_only 时 tool 覆写为 "test"。

### 2. 测试 API

- [ ] http/api/tests.rs（新增）：POST /api/tests handler（构建请求体、调 proxy_request、返响应）。
- [ ] http/api/mod.rs: 注册 tests。
- [ ] http/router.rs: 挂载 /api/tests。

### 3. 前端测试面板

- [ ] lib/api.ts: 增 testsApi + TestRequest/TestResult/SseChunk 类型。
- [ ] pages/RoutesPage.tsx: 每条路由卡片加测试面板（配置区+SSE 结果区+统计栏+fallback 链）。
- [ ] pages/LogsPage.tsx: 加过滤开关（全部/生产/测试）。

### 4. 质量门

```bash
cd src-tauri && cargo check (0 error) && cd .. && npm run build
```

## 验证

- POST /api/tests {route:"v1", path:"/v1/chat/completions", prompt:"Hi", stream:false} → 返回正确 JSON。
- 流式模式 → SSE 逐 chunk 在前端渲染。
- 统计栏显示首 token 时间/chunk 数/总耗时。
- 测试后端点 cooldown_until/last_failure_at 不变。
- 媒体测试 → blob: URL 临时展示。

## 回滚

本任务全是新增模块 + RoutesPage 扩展；回滚即 git checkout。
