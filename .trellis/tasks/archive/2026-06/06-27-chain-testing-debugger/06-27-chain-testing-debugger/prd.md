# 真实链路测试与调试器

## 目标

为 agent-switch 提供真实链路测试与流式调试能力。每条已配置路由（claude-code、codex、v1）可在管理端发起真实链路测试：自定义 prompt、选择模型、模拟完整链路经过 selector→auth→translator→forward 管道，并在 UI 实时展示流式输出、首 token 时间、chunk 数、fallback 链路与 token 用量。Images/Audio 测试在当前会话临时展示。

## 背景与边界

- 父任务：06-26-agent-switch-web-router-mvp，子任务拆分第 7 项。
- 已建成基础：3 条真实转发路由、完整 RouteProxy 管道、request_logs 表、Frontend RoutesPage + LogsPage。

### 父任务已确认的约束

- 测试可自定义 prompt，走完整链路管道，可能消耗 token，UI 须明确提示。
- 测试日志不保存 prompt/messages/完整正文/headers/API Key/OAuth token。
- 测试默认不影响生产故障转移/冷却状态。
- 流式调试器：实时输出、首 token 时间、chunk 数、中断/完成状态、错误摘要、fallback 链路。支持取消。
- Images/Audio 当前会话临时展示，不持久化媒体内容。

## 技术决策

### D1 测试链路 = 走真实 RouteProxy 管道 + test_only 标志（效仿 sub2api）

RouteProxy 接收 test_only: bool 参数。test_only=true 时禁冷却回写、禁故障转移 cooldown 更新、日志标记 tool=test。

### D2 测试入口 = 集成在 RoutesPage

每条路由卡片加测试面板：选 sub-path、填 prompt、选模型、选流式/非流式。测试结果 SSE 实时展示或 JSON 展示。

### D3 Images/Audio 测试展示 = 当前会话 blob URL

前端用 blob: URL 临时展示，不存文件系统/DB。

## 需求

### R1 测试 API

- R1.1 POST /api/tests：{ route, path, model?, prompt, stream: bool }。
- R1.2 管道内 test_only=true：禁冷却回写、禁 cooldown 更新、selector 不排除活跃端点。
- R1.3 响应：流式=SSE，非流式=JSON。

### R2 流式调试器

- R2.1 SSE 逐 chunk 渲染文本/tool_call 累积。
- R2.2 统计面板：首 token 时间、chunk 数、总耗时、token 用量。
- R2.3 fallback 链路可视化：hop 详情。
- R2.4 取消按钮。

### R3 Images/Audio 测试

- R3.1 blob: URL 临时展示 img/audio。
- R3.2 显示 media_type、content_length、耗时。

### R4 测试日志

- R4.1 写入 request_logs 标记 tool=test。
- R4.2 日志页过滤开关。

## 验收标准

- [ ] AC1: claude-code 测试请求 → 正确响应。
- [ ] AC2: codex 测试请求 → 正确响应。
- [ ] AC3: v1/chat/completions 测试请求 → 正确响应。
- [ ] AC4: 流式模式 → SSE 逐 chunk 显示 + 统计面板。
- [ ] AC5: 测试触发的上游错误显示在 fallback 链中，不写冷却。
- [ ] AC6: 图片测试 → blob: URL 临时展示。
- [ ] AC7: 测试不修改端点 cooldown_until 或 last_failure_at。
- [ ] AC8: 取消按钮中断进行中的测试。
- [ ] AC9: 日志页可过滤测试日志。
- [ ] AC10: Token 消耗提示在发送前显示。
- [ ] AC11: 质量门：cargo check (0 error)、npm run build。

## 暂不纳入范围

- 定时/批量测试。
- 测试结果持久化导出。
