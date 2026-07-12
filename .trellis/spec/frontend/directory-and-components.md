# 前端目录与组件约定

## 技术栈

- React 18 + TypeScript strict + Vite 7（`package.json:37-39,84-85`；`tsconfig.json:1-23`）。
- Tailwind CSS 3、Radix UI、shadcn 风格原子组件、CodeMirror、Recharts（`package.json:29-39,42-92`）。
- 路径别名 `@/* → src/*`（`tsconfig.json:16-19`）。

## 目录职责

| 路径 | 职责 | 约束 |
|---|---|---|
| `src/components/ui/` | 通用无业务原子组件 | 不调用 Tauri command，不知道 AppType/Provider |
| `src/components/<domain>/` | Provider、Proxy、MCP、Skills、Sessions 等领域组件 | 只依赖 hooks/API wrapper，不直接拼 command 字符串 |
| `src/pages/` | 页面级组合 | 负责编排，不承载数据库/live 文件算法 |
| `src/hooks/` | 查询、mutation、跨组件交互 | mutation 成功后精确失效 query key |
| `src/lib/api/` | Tauri IPC 领域 wrapper 与事件类型 | command 名和参数映射只集中在这里 |
| `src/lib/query/` | QueryClient 与 query keys | 禁止在组件里散落字符串 key |
| `src/config/` | App/Provider UI 映射与预设 | 删除 App 时必须跨层检查，不能只改数组 |
| `src/types/` | 跨领域 TS 类型 | 与 Rust serde shape 对齐，避免无边界 `any` |
| `src/i18n/` | 语言 runtime 与 locale | 目标固定 `zh`；其他 locale 在专属裁剪批次删除 |

现有前端入口用 `QueryClientProvider` 包裹应用并通过 `invoke` 完成初始化（`src/main.tsx:9-14,79-113`）。

## 命名与组织

- React 组件和文件使用 PascalCase；hooks 使用 `useXxx`；API wrapper 使用 `<domain>Api`。
- 页面不跨目录导入领域内部实现；需要复用时提取到 `components/ui`、`hooks` 或 `lib`。
- 组件 props 显式建模；避免把整个 App state 或 Provider map 透传给深层组件。
- 对话框/表单的草稿状态留在组件内；提交后的 SSOT 是后端/Query cache。

## Provider 预设目录（D22 精选官方）

`src/config/*ProviderPresets.ts`（claude 为规范集，codex/gemini/claudeDesktop/openclaw/opencode/hermes 为按 name 镜像）只保留一手模型厂商官方模板 + 少数特批知名聚合入口（OpenRouter / SiliconFlow / ModelScope）；聚合/中转商预设、返利/来源跟踪参数（`aff=`/`ref=`/`utm_*`/`from=CH_`/邀请码/优惠码）、`isPartner`/`primePartner`/`partnerPromotionKey` 及 `zh.json > providerForm.partnerPromotion` 促销文案已整体清除（2026-07-12，任务 `07-12-ccs-preset-ad-cleanup`）。被保留的官方项若原带渠道参数，一律洗成裸官方链接。`partnerPromotionKey` 由 `ApiKeySection` 以 `{key && t(...)}` 守卫渲染，值移除后不再显示促销；新增预设默认不带任何合作/返利字段，完整 JSON 自定义入口不受影响。

## 单应用目标

ccs 当前 `APP_IDS`/`SKILLS_APP_IDS`/`MCP_APP_IDS` 覆盖多个客户端（`src/config/appConfig.tsx:17-37`）。目标产品收缩到 Claude 后：

- `APP_IDS`、`VALID_APPS`、技能/MCP app 列表只含 `claude`；
- 移除 AppSwitcher 和“可见应用”设置，而不是留下永远只有一个选项的 UI；
- 所有领域组件的 `appId` 参数若只剩 Claude，应评估是否在 API 边界固定，而不是让 UI 到处传常量；
- 仍保留 Provider 的上游协议类型（Anthropic、OpenRouter、Codex OAuth、GitHub Copilot），它们不是客户端 AppId。

## Provider JSON 安全编辑

`Provider.settings_config` 是完整 `settings.json` 快照。表单编辑必须：

1. 克隆原 JSON；
2. 只更新表单拥有的路径；
3. 保留未知顶层键、嵌套对象、数组和 `env` 中的非连接键；
4. 让用户保有裸 JSON 编辑入口；
5. 不把快照缩成 `ANTHROPIC_*` 固定 schema，也不增加 `meta.snapshot`。

每个涉及 Provider 表单的改动都需增加“未知字段往返”测试。
