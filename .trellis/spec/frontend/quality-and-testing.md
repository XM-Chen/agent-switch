# 前端质量与测试

## 必跑门禁

```bash
pnpm install --frozen-lockfile
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
```

这些命令来自 `package.json:5-16`。仓库没有 `pnpm lint` script；旧 CONTRIBUTING 中相关描述已漂移，不得当作有效门禁。

## TypeScript 与格式化

`tsconfig.json:1-23` 启用 strict、`noUnusedLocals`、`noUnusedParameters`、`noFallthroughCasesInSwitch`：

- 不用 `@ts-ignore` 掩盖跨层类型错误；确需临时绕过时说明原因并绑定清理任务。
- API 返回类型必须显式；无法信任的 JSON 在边界用 Zod/类型守卫验证。
- 不为通过 typecheck 留下未用导入、假分支或 `any` 扩散。
- 只用仓库的 Prettier script 格式化 `src/**/*.{js,jsx,ts,tsx,css,json}`（`package.json:12-13`）。

## Vitest

Vitest 使用 jsdom、全局 setup 和 Testing Library/MSW（`vitest.config.ts:4-19`）。

测试层次：

1. 工具/转换纯单元测试；
2. hooks/API wrapper 的 invoke mock 与 query cache 测试；
3. 组件交互测试；
4. `tests/integration/` 的跨组件应用流程。

要求：

- 查询元素优先用 role/name；不要在多处相同文案时滥用 `getByText`。
- 测试必须清理 event listener、timer、localStorage 和 QueryClient，避免相互污染。
- Provider 编辑/切换覆盖未知 JSON 字段往返、Common Config 叠加/剥离后的 UI 行为。
- App 裁剪后删除失效 fixture/测试，同时增加“仓库中无被删 AppId UI/route/invoke”残留扫描。
- 安全确认流程覆盖未确认拒绝、确认持久化、后端失败不误报成功。

## 基线已知失败

原样 ccs v3.16.5 的 `tests/integration/App.test.tsx` 有 4 个 OpenClaw 多匹配失败。权威记录见：

`../../tasks/07-10-ccs-baseline-bootstrap/research/r3-validation-results.md`

不得在其他改动中把既有失败伪装成新回归或静默跳过。OpenClaw 裁剪批次删除相关入口和测试后，`pnpm test:unit` 必须全绿。

## 完成定义

前端改动只有在 typecheck、format、所有相关 Vitest、renderer build 通过，且后端 IPC 契约已联动验证后才算完成。若受原样基线问题阻塞，必须记录具体测试名、失败断言和归属批次。
