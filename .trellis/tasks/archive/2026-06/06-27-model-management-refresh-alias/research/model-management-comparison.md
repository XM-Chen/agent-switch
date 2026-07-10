# 模型管理刷新与别名对比研究

## 研究问题

为 agent-switch 设计模型管理、上游刷新和别名映射能力。

## 四项目对比

| 方面 | 9router | ccs | cpa | sub2api |
|------|---------|-----|-----|---------|
| 模型来源 | 内置 + 自定义 | 供应商配置 + 角色映射 | YAML 配置 | 上游同步 + 手动 |
| 模型别名 | 用户可配置 alias，存储本地 db | display name → model ID 映射 | alias → name 配置，支持别名池 | from → to 映射（预设 + 手动） |
| 别名冲突 | — | — | 建议用户使用唯一别名/前缀 | 检测重复 from 并提示 |
| 自定义模型 | `AddCustomModelModal`，调 `/api/models/custom` | CodexFormFields 手动添加 | — | — |
| 角色映射 | — | ModelMapping: haiku/sonnet/opus/fable → 上游 | — | — |
| 模型能力 | kind 字段（llm） | reasoning 能力类型 | — | — |
| 刷新逻辑 | — | handleFetchModels（手动） | — | syncUpstreamModels（手动 + 自动） |
| 能力类型 | kind filter | reasoning capability | — | — |

## 对 agent-switch 的启发

- 角色映射借鉴 ccs 的 ModelMapping。
- 别名池借鉴 cpa 的 oauth-model-alias。
- 自定义/内置模型分离借鉴 9router。
- 上游同步借鉴 sub2api 的 syncUpstreamModels。
- 能力类型借鉴 9router 的 kind 字段 + agent-switch 的多端点路由需求。
