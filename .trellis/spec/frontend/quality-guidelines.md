# 前端质量规范

## 必跑命令

```bash
npm run build
npm run test
```

## 测试框架

- 使用 Vitest。
- 纯函数、API 参数构造、展示常量必须优先写单元测试。
- 复杂页面行为可后续引入 e2e，但第一版不强制 Playwright。

## 覆盖重点

- LogsPage production/test 过滤参数。
- Dashboard health 聚合、fallback hop 计数、时间格式化。
- 共享 presentation 常量存在且页面不重复定义。
