# Implement: Deep Link 一键导入（ccswitch://v1/import）

## Checklist

1. 添加 deep-link 插件依赖与 `tauri.conf.json` scheme：仅注册 `ccswitch`。
2. 实现 URL event handler：冷启动/运行中都能把 URL 传给前端确认对话框。
3. 新增 DeepLink 类型、parser 与敏感字段脱敏 helper，覆盖 scheme/version/path/resource 校验。
4. 实现 parse/merge/preview API，Base64 解码与 `config` 合并不落库。
5. 实现 provider 导入：Claude/Codex 映射、endpoint 加密、provider 创建、可选确认后 switch。
6. 实现 prompt 导入：Base64 content、Prompts service create/enable、回填保护。
7. 实现 MCP 导入：Base64 JSON、`mcpServers` 批量导入、已存在项冲突/合并策略、即时 sync。
8. 接入 skill 导入：等待 `cc-skills` service 后调用 repo install；未就绪时明确返回 unsupported。
9. 新增前端 DeepLink 确认对话框，展示预览、敏感字段遮蔽、取消/确认/结果。
10. 补测试与构建回归。

## Validation Commands

- `cargo test`
- `npm test -- --run`
- `npm run build`

## Review Gates

- 协议 gate：只注册/接受 `ccswitch://v1/import`，不出现 `agentswitch://`。
- 安全 gate：取消不落库、不联网；日志和 UI 不泄漏完整 API Key/token/config/content。
- 加密 gate：provider 导入 API Key 走 endpoint 加密，不写入 `providers.settings_config` 明文。
- 依赖 gate：`skill` import 必须复用 `cc-skills` service，不能私自实现绕过路径安全的下载器。

## Rollback Points

- Parser/import service 可先以手动粘贴 URL 入口验收，再启用 OS scheme 注册。
- `skill` 资源可在 `cc-skills` 未完成时保持明确 unsupported，不阻塞 provider/prompt/mcp。
- 如 single-instance/deep-link 插件跨平台异常，可先保留前端手动粘贴 URL 作为 fallback。