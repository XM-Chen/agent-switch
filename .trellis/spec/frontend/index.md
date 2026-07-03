# agent-switch React 前端规范

> 适用范围：`src/` 下的 React + TypeScript + Vite 前端应用。

---

## 概览

前端提供 8 个中文页面：总览、账号、端点、模型、工具、路由、日志、设置。数据访问通过 `src/lib/api.ts`，请求状态通过 TanStack Query 管理。

## 规范索引

| 规范 | 内容 |
|------|------|
| [目录结构](./directory-structure.md) | 页面、组件、lib、测试文件布局 |
| [API Client](./api-client-guidelines.md) | `src/lib/api.ts` 参数/响应/错误约定 |
| [状态管理](./state-management.md) | TanStack Query queryKey、loading/error 空态 |
| [组件规范](./component-guidelines.md) | 中文 UI、共享展示常量、死代码处理 |
| [质量规范](./quality-guidelines.md) | build/test 命令与 Vitest 约定 |

Trellis 平台适配规范已迁移到 `../trellis-runtime/`。

## 必跑命令

```bash
npm run build
npm run test
```
