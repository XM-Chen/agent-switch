# 模型别名与模型映射策略对比研究

## 研究问题

为 agent-switch 第一版设计模型别名与模型映射能力，需要综合参考：

- `9router`：模型前缀、provider/model 显式路由、用户可配置 alias、模型规范化与组合/回退链。
- `ccs`（cc-switch）：Claude Code / Claude Desktop 角色模型映射，Codex 模型映射与模型目录生成。
- `cpa`（CLIProxyAPI）：配置驱动的模型 alias、prefix、API Key / OAuth 维度别名与“首个匹配/首个别名优先”语义。
- `sub2api`：账号/渠道级模型映射、请求模型/上游模型/实际模型/计费模型区分、冲突检测与通配规则。

## 四项目可借鉴点

| 项目 | 做法 | 对 agent-switch 的启发 |
|------|------|------------------------|
| `9router` | 支持 `provider/model` 显式模型名、provider alias、模型前缀推断、用户配置的模型 alias，以及 combo model / fallback chain。MITM 场景中还有 `MODEL_NO_MAP`、模型同义词和正则模式匹配。 | 第一版应支持“显式端点/供应商前缀优先”，并提供不可映射/直通保护；但正则模式匹配第一版只作为预留，不默认暴露复杂 UI。 |
| `ccs` | 对 Claude Code / Claude Desktop 采用角色模型映射，例如 `sonnet`、`opus`、`haiku`、`fable` 等角色映射到真实上游模型；Codex 侧也有模型映射表与模型目录生成。 | 第一版必须有工具级角色映射，尤其是 Claude Code 角色模型到具体端点模型的映射；`fable` 等新角色要可扩展，不应写死成三个角色。 |
| `cpa` | `config.example.yaml` 中可为上游模型定义 alias；支持 prefix 让客户端通过 `prefix/model` 定位特定 provider/credential；OAuth 全局 alias 与认证 JSON 内 alias 存在覆盖关系；同名 alias 可形成模型池。 | 第一版应借鉴“显式 prefix”和“别名池”思想，但为了可解释性，UI 中必须显示 alias 解析到的候选端点顺序，不能让同名 alias 隐式随机。 |
| `sub2api` | 支持账号/渠道级模型映射，区分 requested model、upstream model、actual model、billing model source；具备模型映射冲突检测和通配规则。 | 第一版日志和调试器应记录请求模型、解析后的本地别名、上游模型、实际命中端点；保存配置时要做冲突检测，避免同一作用域下 alias 重复导致不可解释路由。 |

## 推荐给 agent-switch 第一版的落地方案

### 1. 核心概念

第一版将“模型”与“别名”分开建模：

- **端点模型（Endpoint Model）**：绑定到具体端点的模型，来源可以是 `synced` 或 `custom`。
- **模型别名（Model Alias）**：用户或工具侧请求使用的模型名/短名/角色名，解析后指向一个或多个端点模型。
- **工具角色映射（Tool Role Mapping）**：模型别名的一种特殊作用域，主要服务 Claude Code 的 `sonnet` / `opus` / `haiku` / `fable` 等角色。
- **显式前缀模型名（Explicit Prefixed Model）**：例如 `endpoint_slug/model_name` 或后续 `provider_slug/model_name`，用于绕过普通 alias 解析，直接指定候选范围。

### 2. 第一版数据模型建议

建议 SQLite 中至少预留以下字段：

#### `endpoint_models`

- `id`
- `endpoint_id`
- `model_name`
- `display_name`
- `source`: `synced | custom`
- `capabilities_json`: 上下文长度、是否支持流式、是否支持工具调用等，可为空。
- `last_seen_at`
- `is_available`
- `created_at`
- `updated_at`

唯一约束：`endpoint_id + model_name`。

#### `model_aliases`

- `id`
- `scope_type`: `global | tool | route | endpoint`
- `scope_id`: 全局为空；工具/路由/端点时填对应 ID。
- `alias_name`: 用户请求的本地模型名、短名或角色名。
- `target_endpoint_id`
- `target_model_name`
- `priority`
- `enabled`
- `description`
- `created_at`
- `updated_at`

建议唯一约束：`scope_type + scope_id + alias_name + target_endpoint_id + target_model_name`，并在保存时检测同一作用域下同名 alias 的优先级是否重复。

### 3. 解析优先级

第一版采用可解释的固定优先级：

1. **不可映射/直通规则**：系统保留模型或明确标记为 no-map 的模型不做 alias 解析。
2. **显式端点前缀**：如请求 `endpoint_slug/model_name`，优先解析到该端点上的模型。
3. **工具级角色映射**：例如 Claude Code 的 `sonnet`、`opus`、`haiku`、`fable`。
4. **路由级 alias**：某条路由内定义的 alias，只影响该路由。
5. **端点级 alias**：某端点内定义的 alias，用于端点自己的上游模型短名映射。
6. **全局 alias**：兜底 alias。
7. **原名匹配**：如果没有 alias，使用请求模型名在候选端点模型中按原名匹配。
8. **失败**：无匹配时返回可解释错误，列出作用域、请求模型和候选端点，不静默改用其他模型。

### 4. 多目标 alias 与故障转移关系

第一版允许一个 alias 映射到多个目标模型，但必须显式排序：

```text
alias: sonnet
1. endpoint_a / claude-sonnet-4-5
2. endpoint_b / claude-3-7-sonnet
3. endpoint_c / custom-sonnet-compatible
```

解析结果不是单个模型，而是候选链：

```text
requested_model -> resolved_alias -> candidate endpoint models -> failover policy
```

故障转移仍由已确认的“优先级顺序 + 安全错误分类 + 轻量冷却 + 最大尝试次数”控制。alias 只负责生成候选端点模型链，不直接改变生产冷却状态。

### 5. 与 `synced` / `custom` 模型刷新关系

- `synced` 模型来自上游模型刷新，本次刷新未返回时可从端点当前模型列表中删除。
- `custom` 模型由用户手动添加，必须绑定端点，不受刷新删除影响。
- alias 可以指向 `synced` 或 `custom` 模型。
- 如果 alias 指向的 `synced` 模型被刷新删除，alias 不应被自动删除；应标记为 `invalid_target`，UI 显示失效并要求用户重新选择。
- 如果 alias 指向 `custom` 模型，刷新不会影响 alias 有效性。

### 6. 冲突处理

第一版保存 alias 时必须做冲突检测：

- 同一作用域下，同名 alias 可以有多个目标，但必须有唯一 priority。
- 同一作用域下，同名 alias 的多个目标会被视为候选池，不是配置错误。
- 不同作用域可以有同名 alias，通过解析优先级决定谁生效。
- 如果显式前缀与 alias 同名，显式前缀优先。
- 如果 no-map 命中，则所有 alias 都不生效。
- 第一版不默认提供正则 alias 和通配 alias；可以在数据模型中预留 `match_type`，但 UI 暂不开放，避免冲突不可解释。

### 7. 日志与调试展示

请求摘要日志和真实链路测试结果应区分：

- `requested_model`：客户端原始请求模型名。
- `resolved_alias`：命中的 alias；未命中则为空。
- `resolved_scope`：命中的作用域。
- `target_endpoint_id`：最终尝试的端点。
- `upstream_model`：发送给上游的模型名。
- `fallback_chain`：每次尝试的端点、上游模型、错误分类和耗时。

仍然不保存 prompt、messages、完整请求/响应正文、完整 headers、API Key 或 OAuth token。

## 结论

agent-switch 第一版的最优折中是：

> 学习 `ccs` 的工具角色模型映射，学习 `9router` 的显式 provider/model 与 no-map/规范化思想，学习 `cpa` 的 prefix 和别名池，学习 `sub2api` 的账号/端点级映射、冲突检测和可解释日志。第一版落地为“端点模型 + 分作用域 alias + 显式解析优先级 + 有序候选链 + 失效提示”，暂不开放复杂正则/通配 alias。
