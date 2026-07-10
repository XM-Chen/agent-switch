# 规划 agent-switch Web 管理与本地路由 MVP

## 目标

规划 agent-switch 第一阶段 MVP：提供一个本地 Web 管理界面，用于管理上游供应商与常见 AI 编程工具配置，并提供本地路由服务，实现基础自动故障转移。

## 背景与愿景

agent-switch 的长期目标是成为一个 agent 管理与路由应用。它应借鉴以下项目的思路，但不直接照搬：

- `9router`：参考 Web 管理与上游供应商/路由管理体验。
- `ccs`（cc-switch）：参考 Claude Code、Codex、OpenCode 等 AI 编程工具的配置管理与切换体验。
- `cpa`（cli-proxy-api）：参考 CLI 到 API 的代理/转发/兼容层设计。
- `sub2api`：参考订阅/上游来源转换为 API 供应商配置的思路。

当前用户明确希望：

- 第一步就包含类似 `9router` 的网页版前端。
- Web 内容不是普通模型列表，而是更像 `ccs`：管理 Claude Code、Codex、OpenCode 等工具的供应商/配置。
- 支持启用本地路由，用于自动故障转移。
- 默认服务地址为 `http://127.0.0.1:42567`。
- 第一版采用单体应用架构，Web UI、管理 API、本地代理路由、配置存储和日志查看由同一个本地进程承载。
- 第一版采用单端口与路径隔离：`/` 提供 Web UI，`/api/*` 提供管理 API，`/claude-code/*` 提供 Claude Code 路由，`/codex/*` 提供 Codex 路由，`/v1/*` 作为 OpenAI-compatible 入口，`/health` 提供健康检查。
- 第一版 `/v1/*` OpenAI-compatible 入口采用较完整的多端点真实转发目标，参考 `cpa` / `sub2api` 向完整 API 网关演进的方向：除 `/v1/chat/completions` 与 `/v1/models` 外，还必须真实转发 `/v1/responses`、`/v1/embeddings`、`/v1/images`、`/v1/audio` 等常见 OpenAI-compatible 端点。该选择会显著扩大第一版协议适配和测试矩阵；设计与实现计划必须把这些端点拆成独立可验收项，并明确各端点的上游能力要求、失败语义、日志字段和测试用例，避免把多端点支持写成无法验证的笼统承诺。
- 实现应从简单到困难推进，不必一步到位。
- 所有项目文档默认使用中文。
- 第一版 Web UI 界面语言默认使用中文，包括导航、表单、提示、错误信息、配置说明和接管风险提示。
- 第一版 Web UI 信息架构确认使用 8 个中文页面：总览、账号、端点、模型、工具、路由、日志、设置。
- 第一版需要新增模型管理/模型映射能力；模型列表不能仅依赖一次性手动配置，必须支持根据上游渠道自动刷新模型列表，并用于 Claude Code 角色映射、Codex 模型前缀/模型名路由和端点能力展示。
- 第一版模型列表更新采用手动刷新、应用启动时自动刷新、定时刷新三种方式；只刷新启用端点，刷新失败不能阻塞应用启动或影响已有配置，需记录 `last_model_sync_at` 和 `last_model_sync_error`。
- 自动刷新上游模型列表必须有总开关，默认关闭；只有用户开启后才执行启动自动刷新和定时刷新。
- 模型页面需要提供“一键全局刷新上游渠道模型”按钮，用于用户主动刷新所有启用上游渠道的模型列表。
- 模型列表定时刷新以 6 小时为基准周期，并加入抖动机制，避免刷新间隔过于规律而被上游判定为机器人行为；定时刷新仅在应用运行期间执行。
- 模型刷新允许不同上游渠道并发；同一 host 默认最多 1 个刷新任务，同一账号/凭据默认最多 1 个刷新任务。手动刷新不排队不同 host，定时刷新使用同样的 host/凭据分组限流；刷新失败不影响已有模型列表。
- 模型刷新采用上游列表覆盖语义：本次刷新未返回的旧模型视为已下线并从该端点当前模型列表中删除；删除前不得破坏用户的路由/模型映射，若映射引用了被删除模型，UI 必须显示映射失效提示并要求用户重新选择。
- 第一版允许用户手动添加自定义模型；自定义模型需要标记来源为 `custom`，不受上游模型刷新机制影响，不会因为上游刷新未返回而被删除或覆盖。
- 第一版自定义模型必须绑定到具体端点，不提供未绑定端点的全局自定义模型；如多个端点需要同名自定义模型，需分别添加到对应端点。
- 第一版必须支持模型别名能力，用于把本地模型名/短名/工具侧模型名映射到具体端点的上游模型；模型别名方案综合借鉴 `ccs` 的工具角色模型映射、`9router` 的显式 provider/model 与 no-map/规范化思想、`cpa` 的 prefix 和别名池、`sub2api` 的账号/端点级映射与冲突检测，第一版落地为“端点模型 + 分作用域 alias + 显式解析优先级 + 有序候选链 + 失效提示”。
- 第一版模型别名解析优先级确认采用：no-map/直通规则 > 显式端点前缀 > 工具级角色映射 > 路由级 alias > 端点级 alias > 全局 alias > 原名匹配 > 可解释失败；第一版不默认开放正则 alias 或通配 alias，只预留数据模型扩展位。
- 第一版允许一个 alias 映射到多个目标端点模型，但必须通过显式优先级形成有序候选链；alias 只生成候选链，实际失败切换仍遵循已确认的故障转移策略。
- alias 可以指向 `synced` 或 `custom` 端点模型；如果指向的 `synced` 模型被刷新删除，alias 不自动删除，而是标记失效并在 UI 中提示用户重新选择；指向 `custom` 模型的 alias 不受刷新影响。
- 请求摘要日志和真实链路测试结果需要区分并展示 `requested_model`、`resolved_alias`、`resolved_scope`、`target_endpoint_id`、`upstream_model` 与 `fallback_chain`，仍不得保存 prompt、messages、完整请求/响应正文、完整 headers、API Key 或 OAuth token。
- 第一版模型管理必须引入模型能力类型，并在路由前强制过滤；每个端点模型至少需要标记 `chat`、`responses`、`embeddings`、`images`、`audio`、`streaming`、`tool_calling`、`vision_input` 等能力。`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings`、`/v1/images`、`/v1/audio` 等入口只能选择具备对应能力或明确兼容转换能力的模型；alias 保存和路由解析时也必须校验能力，避免把文本 alias 指向 embedding/image/audio-only 模型，或把 image/audio alias 指向 chat-only 模型。
- 第一版不做首次启动向导；首次打开不使用强制 wizard 流程。页面可以通过空状态提示和普通操作入口引导用户，但不能阻塞用户进入主界面。

## 已确认事实

- 本项目自身远程仓库为 `origin -> git@github.com:XM-Chen/agent-switch.git`。
- 参考项目远程仓库已配置：
  - `ref-9router`
  - `ref-cc-switch`
  - `ref-cli-proxy-api`
  - `ref-sub2api`
- Trellis/agent 相关生成文件已加入 `.gitignore`。
- 项目尚未实现业务代码，目前仍处于产品和架构规划阶段。
- 第一版技术栈已决定采用 Tauri + Rust + Web 前端，优先参考 `ccs` 的桌面应用形态。
- 第一版不做系统托盘和后台常驻；窗口打开时本地路由服务运行，关闭窗口即停止服务。架构上预留后续托盘/后台常驻扩展。
- 第一版支持自动接管本地工具配置，但必须由用户显式开启；全局默认关闭，每个 Agent/工具（Claude Code、Codex，后续 OpenCode）也都默认关闭。
- 自动接管设置需要按工具分别持久化，用户修改后记住最后一次设置。
- 开启某个工具的自动接管后，应用可自动写入该工具配置，使其指向 agent-switch 本地路由；关闭自动接管后，不再主动修改该工具配置。
- 关闭某个工具的自动接管后，仅停止后续自动写入，不自动恢复开启前配置；如果该工具配置仍指向 agent-switch，本版本不静默改回。
- 自动接管同步时机参考 `ccs`：用户打开某个工具的自动接管开关后立即写入一次；之后只有当用户在 agent-switch 内更改相关配置并点击保存时，才自动同步写入该工具配置。
- 第一版自动接管写入 Claude Code / Codex 配置前必须备份原始配置；备份用于人工查看和手动恢复，不改变“关闭自动接管后不自动恢复原配置”的既定语义。备份记录至少包含工具名、原配置路径、备份文件路径、备份时间和接管写入目标；UI 需要提供查看备份位置或复制恢复说明的入口。
- 第一版存储策略学习 `ccs`：核心配置、供应商、端点、路由规则、自动接管状态、API Key、Codex OAuth token 和请求摘要日志存 SQLite；设备级 UI 偏好存 Tauri store/JSON；运行日志写日志文件。
- 第一版敏感凭据保护采用“SQLite 加密字段 + 系统 Keychain/凭据管理器保存主密钥”：API Key、OAuth access token、refresh token 等敏感字段在 SQLite 中必须加密后存储；主密钥保存到 Windows Credential Manager、macOS Keychain 或 Linux Secret Service/libsecret。系统凭据管理器不可用时不得静默降级为明文存储，应在 UI 中明确提示并要求用户处理；数据库迁移到另一台机器后，凭据可能需要重新录入或重新登录。
- 第一版支持完整加密导入/导出包，允许包含 API Key、OAuth access token、refresh token 等敏感凭据；该能力属于偏密码管理器/完整备份工具方向的高级方案，不是四个参考项目的主线简单做法。导出包必须强制加密并明确提示风险；不得提供未加密导出敏感凭据的路径。设计中需要定义导出包版本、加密算法、密码/密钥策略、弱密码提示、导入冲突处理、导入后是否立即启用端点和自动接管等规则。
- 第一版支持两种导出模式：
  - 本机加密完整备份：使用系统 Keychain/凭据管理器主密钥加密，允许包含 API Key / OAuth token，适合本机恢复或同一系统凭据环境恢复，不保证跨机器恢复敏感凭据。
  - 可迁移脱敏配置导出：不包含 API Key、OAuth token、Authorization header、系统主密钥、请求日志、媒体内容或自动接管备份文件；包含账号/端点非敏感元数据、base URL、协议类型、启用状态、优先级、custom 模型、alias、路由规则和可迁移 UI 配置。导入后凭据进入缺失状态，需要用户重新录入 API Key 或重新登录 OAuth。
- 加密导出包使用系统 Keychain/凭据管理器主密钥加密，不要求用户为每次导出单独设置导出密码；该方案参考 `ccs` 的系统凭据管理器思路，但会使包含敏感凭据的完整导出包绑定当前机器或同一系统凭据环境。UI 必须明确提示：此导出包主要用于本机安全备份/同一系统环境恢复，不保证跨机器恢复敏感凭据；如果系统凭据管理器中的主密钥丢失或不可访问，导出包中的敏感凭据无法解密。
- 加密导出包导入后恢复账号、端点、API Key/OAuth token、模型、alias、路由规则和端点启用状态；但自动接管开关不得自动恢复为开启，也不得在导入完成后自动写入 Claude Code / Codex 配置。导入后的自动接管状态应统一关闭，或显示为“曾开启，需重新确认”，由用户手动检查后重新开启。
- 第一版请求日志学习 `sub2api`：默认只记录请求摘要、路由轨迹、耗时、错误、token 用量和请求体 hash；不记录 prompt、messages、完整请求/响应正文、完整 headers、API Key 或 OAuth token。Debug 详细日志可预留但默认关闭。
- 第一版故障转移策略已确认：综合学习 `ccs`、`9router`、`cpa`、`sub2api`，采用“优先级顺序 + 安全错误分类 + 轻量冷却 + 最大尝试次数 + fallback 链路摘要日志”。默认对网络错误、超时、408、429、529、5xx 和容量类错误切换；默认不对普通 400/405/406/413/414/415/422/501、无效请求、上下文超限、本地配置/数据库错误、已开始输出的流式响应切换；401/403、404/model_not_found、余额不足按账号/端点类型谨慎处理。
- 第一版端点测试改为学习 `cpa`：提供管理端真实链路测试调用能力，用于验证 base URL、API Key/OAuth 凭据、路径、模型和协议。测试可以自定义 prompt，并模拟真实完整链路请求，经过 agent-switch 的路由、认证注入、协议适配和可选故障转移后请求真实上游。测试可能消耗 token，UI 必须明确提示；测试日志仍遵守 `sub2api` 风格，默认不保存 prompt/messages/完整响应正文/API Key/OAuth token。测试请求默认不影响生产故障转移/冷却状态，只记录测试结果和测试链路。
- 第一版真实链路测试支持完整流式调试器：用户可选择流式模式，UI 实时展示流式输出片段、首 token 时间、chunk 数量、流式完成/中断状态、错误摘要和 fallback 链路；流式响应一旦开始输出后禁止继续 fallback；测试 UI 必须支持取消请求，且默认不持久化完整流式内容。
- 第一版 Images/Audio 日志与调试展示采用“当前调试会话临时展示，默认不持久化媒体内容”：真实链路测试和调试器可以临时展示图片缩略图、音频播放控件、响应摘要、媒体大小、MIME、耗时等；默认不得持久化图片/音频文件、base64、完整响应 body、语音文本或媒体输入。摘要日志只允许保存 endpoint、model、media_type、content_length、hash、status、latency、error_kind、fallback_chain 等非正文信息；UI 可提供“另存为”按钮，由用户主动保存当前结果到用户选择的位置。
- 第一版本地服务安全边界采用极简方案：所有入口默认不做本地 token/session 认证，仅绑定 `127.0.0.1`。该方案实现和工具接入最简单，但需要在设计中明确风险：本机其他进程理论上可以访问 `/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 并可能修改配置或消耗上游额度；后续版本应预留本地访问认证能力。

## 第一阶段范围草案

### 必须包含

- 本地 Web 管理界面。
- 上游供应商管理需要暴露两层模型：账号/供应商组层，以及端点层。
- 账号登录/账号管理部分优先学习 `9router`：面向账号或供应商组管理认证、登录状态、可用性与关联端点。
- 第一版 OAuth provider 仅实现 OpenAI Codex OAuth provider，参考 `9router` 的 Codex OAuth provider 元数据与登录流程。
- 主服务仍固定为 `http://127.0.0.1:42567`；Codex OAuth 登录期间允许临时启动参考 `9router` 的专用 callback 端口（例如固定端口 + `/auth/callback`），登录结束后释放。
- 第一版 OAuth 范围不扩展到 Anthropic、Google、OpenAI 通用 OAuth 或其他网页登录；其他账号类型先通过 API Key / Token 或后续版本处理。
- 端点管理部分优先学习 `ccs`：面向具体 base URL、API Key、协议格式、模型映射、优先级、启用状态和故障转移配置。
- Claude Code / Codex / OpenCode 相关配置管理入口。
- OpenCode 第一版仅提供手动配置说明和预留入口，不做自动接管；可引导用户使用 `/v1` 或后续兼容入口手动接入。
- 第一版最小可用代理闭环必须同时覆盖 Claude Code 与 Codex。
- Claude Code 的路由逻辑优先参考 `ccs`（cc-switch）。
- Codex 的路由逻辑优先参考 `9router`。
- 本地服务地址默认固定为 `http://127.0.0.1:42567`。
- 本地路由开关。
- 基础自动故障转移概念。

### 倾向先简化

- 第一版只做本地单机管理，不做云同步。
- 第一版优先做配置管理和路由骨架，不追求完整协议转换。
- 第一版自动故障转移可以先基于手动排序和简单失败切换，不做复杂评分/测速。

## 子任务拆分

由于第一版范围已从轻量 MVP 扩展为较完整的本地桌面网关产品，当前父任务只保留总体愿景、跨模块约束和集成验收；后续实现应拆成多个可独立规划、实现和验收的子任务。

已创建子任务：

1. `.trellis/tasks/06-27-app-shell-local-service`：应用骨架与本地服务。
   - 范围：Tauri + Rust + Web、单进程单端口、路径隔离、SQLite 初始化、`/health`。
2. `.trellis/tasks/06-27-accounts-endpoints-credential-security`：账号端点与凭据安全。
   - 范围：账号/供应商组、端点管理、SQLite 加密字段、系统 Keychain/凭据管理器主密钥。
3. `.trellis/tasks/06-27-model-management-refresh-alias`：模型管理刷新与别名。
   - 范围：synced/custom 模型、模型能力类型、刷新机制、alias 解析与冲突处理。
4. `.trellis/tasks/06-27-tool-takeover-claude-code-codex`：Claude Code 与 Codex 工具接管。
   - 范围：自动接管开关、配置写入、写入前备份、OpenCode 手动配置说明。
5. `.trellis/tasks/06-27-routing-failover-core`：路由与故障转移核心。
   - 范围：Claude Code 路由、Codex 路由、优先级、冷却、错误分类、fallback 链路日志。
6. `.trellis/tasks/06-27-openai-compatible-v1-endpoints`：OpenAI-compatible v1 多端点。
   - 范围：`/v1/chat/completions`、`/v1/models`、`/v1/responses`、`/v1/embeddings`、`/v1/images`、`/v1/audio` 真实转发。
7. `.trellis/tasks/06-27-chain-testing-debugger`：真实链路测试与调试器。
   - 范围：自定义 prompt、完整链路测试、流式调试器、Images/Audio 当前会话临时展示。
8. `.trellis/tasks/06-27-import-export-settings`：导入导出与设置。
   - 范围：本机加密完整备份、可迁移脱敏配置导出、导入冲突处理、设置页面。

依赖规则：父子结构只表示交付拆分，不自动表达依赖；如果某个子任务依赖另一个子任务，必须写入对应子任务的 `prd.md` / `implement.md`。

## 待澄清问题

1. 第一版 Web 应该做“纯配置管理控制台”，还是必须同时实现可用的本地代理转发？
   - 已决定：第一版必须包含最小可用本地代理闭环。
   - 范围约束：代理能力只要求覆盖一个最小工具链路，避免第一阶段同时处理所有协议和所有工具。

## 初步验收标准

- [ ] 形成用户认可的 MVP 范围。
- [ ] 明确第一版 Web 管理界面包含哪些页面。
- [ ] 明确 Claude Code / Codex / OpenCode 的第一版管理深度。
- [ ] 明确本地路由的第一版协议范围与故障转移规则。
- [ ] 明确默认服务地址 `http://127.0.0.1:42567` 的约束。
- [ ] 产出中文 `design.md` 和 `implement.md`，用于后续实现任务。

## 暂不纳入范围

以下内容是否纳入后续阶段，待规划确认：

- 第一版不做完整 Claude/Codex/OpenCode 多协议双向转换；但 `/v1/*` OpenAI-compatible 入口按较完整多端点兼容方向规划，端点级真实支持范围需在设计阶段拆分。
- 多用户登录与云端同步。
- 自动测速、成本优化、智能调度。
- 订阅源导入与批量转换。
- 浏览器外公网暴露或远程访问。
