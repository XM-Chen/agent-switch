# React 前端目录结构

```text
src/
├── App.tsx                    路由入口
├── main.tsx                   React Query provider 与渲染入口
├── components/                可复用组件
│   ├── layout/                AppShell 等布局组件
│   ├── models/                模型/别名相关组件
│   ├── providers/             切换器 provider 卡片/分组/表单
│   └── tools/                 工具接管卡片
├── lib/
│   ├── api.ts                 管理 API client
│   ├── format.ts              共享格式化函数
│   └── presentation.ts        工具/分类展示标签与颜色
└── pages/                     9 个中文页面与页面专属纯函数/测试
```

## 规则

- 页面级纯函数如果需要测试，提取到 `pages/*Utils.ts`。
- 共享展示常量放到 `src/lib/presentation.ts`，禁止在多个页面重复定义。
- 共享格式化函数放到 `src/lib/format.ts`。
- 无引用组件/工具函数必须删除或接回真实调用点。
- 单个领域（如 `providers/`）的卡片/分组/表单组件集中到 `components/<domain>/`，与 `pages/<Domain>Page.tsx` + `pages/<domain>Utils.ts` 一一对应，避免散落到 `components/` 根。
