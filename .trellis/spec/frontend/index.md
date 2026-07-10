# 前端规范索引

> 适用基线：cc-switch v3.16.5（`8d1b3306d`）上的 `agent-switch-ccs` 分支。
> 目标产品：Windows、简体中文、Claude Code 单应用的 Agent Switch。

## 必读顺序

1. [目录与组件约定](directory-and-components.md)
2. [IPC、API 与查询状态](ipc-api-and-state.md)
3. [前端质量与测试](quality-and-testing.md)

## 当前事实与目标状态

ccs v3.16.5 当前前端仍支持 `claude`、`claude-desktop`、`codex`、`gemini`、`opencode`、`openclaw`、`hermes` 七类应用（`src/config/appConfig.tsx:17-37`），并由 `App.tsx` 中的 `VALID_APPS` 和 `AppSwitcher` 驱动多应用界面（`src/App.tsx:124-136,1370`）。

Agent Switch 目标状态只暴露 `claude`。这不是只隐藏 AppSwitcher：每次裁剪都必须同步清理 AppId、API wrapper、query key、条件渲染、live 状态、预设、图标、i18n 和测试。跨层删除清单见 `../guides/single-app-trimming.md`。

## 前端不变量

- 页面和组件不能直接拼接后端配置路径或数据库结构；通过领域 API wrapper 调用 Tauri command。
- 远端/后端状态由 TanStack Query 管理；纯 UI 偏好才放 localStorage/context。
- Provider 的 `settings_config` 是任意 JSON 全文快照；结构化表单只能修改自己拥有的字段，不得重建整个对象而丢失未知字段。
- 用户可见新增文案只写简体中文。i18n runtime 在裁剪完成前可暂留，但 active locale 最终固定 `zh`。
- GitHub Copilot 与 ChatGPT Codex OAuth 是 Claude Provider 的托管上游，不能因名字含 Codex/Copilot 而删掉其 Claude 表单和认证入口。
- ccs `AgentsPanel` 只是占位页，目标产品删除；Sessions 中 subagent 日志展示不受影响。

## 证据优先级

出现冲突时按以下顺序判断：

1. 当前源码和测试；
2. `package.json` / `tsconfig.json` / `vitest.config.ts`；
3. CI workflow；
4. README / CONTRIBUTING（可能陈旧）。

旧 Agent Switch 0.2.2 规范只在 `../legacy-agent-switch-0.2.2/` 作为历史参考，不得指导当前实现。
