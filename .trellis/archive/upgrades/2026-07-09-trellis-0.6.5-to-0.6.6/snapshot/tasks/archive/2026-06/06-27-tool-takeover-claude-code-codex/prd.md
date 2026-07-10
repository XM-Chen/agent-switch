# Claude Code 与 Codex 工具接管

## 目标

为 agent-switch 提供「工具自动接管」能力:在用户显式开启后,把本地 AI 编程工具(Claude Code、Codex)的配置文件改写为指向 agent-switch 本地服务(`/claude-code`、`/codex`),写入前备份原配置,并在工具页展示每个工具的当前指向与接管状态。OpenCode 第一版仅提供手动配置说明,不做自动接管。

## 背景与边界

- 父任务:`06-26-agent-switch-web-router-mvp`,本子任务对应父 PRD 第 58-63 行的「自动接管」约束与子任务拆分第 4 项。
- 本地服务固定为 `http://127.0.0.1:42567`,路径隔离:Claude Code 走 `/claude-code`,Codex 走 `/codex`。
- 已建成基础:迁移到 v3(accounts/endpoints/endpoint_models/model_aliases/app_metadata);凭据加密(crypto/keychain);8 个中文页面骨架,其中 `ToolsPage` 仍是占位。
- 参考:接管机制与配置字段参考 `ccs`(cc-switch)的 Claude Code / Codex 接管实现;Codex provider 元数据参考 `ccs` 与 `9router`。

### 范围确认(已与用户确认)

- **本子任务范围 = 配置接管**:开关、配置写入、写入前备份、当前指向检测、工具页 UI。
- **不含真正的代理路由转发**:`/claude-code/*`、`/codex/*` 的实际转发逻辑属于子任务 5 `routing-failover-core`。接管完成后工具配置已指向 agent-switch,但在子任务 5 完成前,实际请求可能返回 501(当前占位)。
- 工具页**读取并解析工具真实配置文件**,显示「当前指向:agent-switch / 官方 / 第三方 / 未配置」。
- 关闭接管后**仅显示备份位置 + 可复制的恢复说明**,不提供任何写回工具配置的按钮(无一键恢复)。

## 需求

### R1 每工具接管开关与持久化

- R1.1 Claude Code、Codex 各自独立的接管开关,全局与每工具默认**关闭**。
- R1.2 开关状态持久化到 SQLite;用户修改后记住最后一次设置。
- R1.3 OpenCode 不提供接管开关,仅提供手动配置说明与预留入口。

### R2 接管写入(开启时)

- R2.1 开启某工具接管时,**立即写入一次**该工具配置,使其指向 agent-switch 本地路由。
- R2.2 写入采用**合并语义**,保留工具配置中与接管无关的其它字段/配置项。
- R2.3 写入的鉴权令牌使用**固定占位符**,绝不把真实上游 API Key / OAuth token 写入工具配置文件。
- R2.4 Claude Code 写入目标:`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` 指向 `http://127.0.0.1:42567/claude-code`,`env.ANTHROPIC_AUTH_TOKEN` 为占位符。
- R2.5 Codex 写入目标:`~/.codex/config.toml` 顶层 `model_provider` 与 `[model_providers.<provider>]`(`base_url` 指向 `http://127.0.0.1:42567/codex`),`~/.codex/auth.json` 的 `OPENAI_API_KEY` 为占位符。
- R2.6 目标配置文件或目录不存在时,创建后再写(支持工具全新安装场景);并把「原文件不存在」作为可识别的备份标记记录。
- R2.7 写入失败不得使应用崩溃;失败需记录错误并在 UI 提示,接管开关状态须反映失败结果。

### R3 写入前备份

- R3.1 每次接管写入前,先把原始配置文件**完整备份**到 agent-switch 应用数据目录下的备份目录。
- R3.2 备份记录至少包含:工具名、原配置路径、备份文件路径、备份时间、接管写入目标(base URL)。
- R3.3 备份记录持久化,可供工具页列出与查看。
- R3.4 已是 agent-switch 接管态的配置(检测到占位符/我方 base URL)再次写入时,不得用接管配置覆盖掉「好的原始备份」。

### R4 同步时机(参考 ccs)

- R4.1 开启接管开关后立即写入一次(R2.1)。
- R4.2 之后仅当用户在 agent-switch 内更改接管相关配置并保存时,才自动重新写入;提供可重复调用、幂等的「重新应用接管」能力。
- R4.3 关闭接管后,停止后续一切自动写入;**不自动恢复**开启前配置;若工具配置仍指向 agent-switch,本版本不静默改回。

### R5 当前指向检测

- R5.1 工具页读取并解析每个工具的真实配置文件,判定当前指向类别:`agent-switch`(我方 base URL)/ `官方`(官方默认 URL 或空)/ `第三方`(其它 URL)/ `未配置`(文件/字段缺失)。
- R5.2 检测为只读操作,不修改任何工具配置。
- R5.3 检测解析失败(文件损坏等)须降级为可读的「无法识别」状态,不崩溃。

### R6 工具页 UI(中文)

- R6.1 三张工具卡片:Claude Code(自动接管)、Codex(自动接管)、OpenCode(手动说明)。
- R6.2 每张自动接管卡片展示:接管开关、当前指向徽标、最近写入时间、最近错误(若有)、备份位置列表 + 可复制的恢复说明。
- R6.3 OpenCode 卡片展示手动配置说明与可复制片段,无开关。
- R6.4 接管开启与写入需有明确的**风险提示**文案(会改写本机工具配置、关闭不自动还原)。
- R6.5 所有文案中文。

## 验收标准

- [ ] AC1:Claude Code / Codex 各有独立接管开关,默认关闭,状态持久化并在重启后保留。
- [ ] AC2:开启 Claude Code 接管后,`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` 指向 `http://127.0.0.1:42567/claude-code`,`ANTHROPIC_AUTH_TOKEN` 为占位符,且原有其它键被保留。
- [ ] AC3:开启 Codex 接管后,`~/.codex/config.toml` 的 `model_provider` 与 `[model_providers.<provider>].base_url` 指向 `http://127.0.0.1:42567/codex`,`~/.codex/auth.json` 的 `OPENAI_API_KEY` 为占位符,`config.toml` 中用户其它配置与 `auth.json` 的 `tokens`/`last_refresh`(若有)被保留。
- [ ] AC4:任何接管写入前都生成备份;备份记录含工具名/原路径/备份路径/时间/写入目标,可在工具页查看。
- [ ] AC5:工具配置文件不存在时,开启接管能创建并写入成功。
- [ ] AC6:已是接管态时再次写入,不覆盖已有的好备份(R3.4)。
- [ ] AC7:关闭接管后不再写入工具配置,也不自动还原;工具配置维持接管态。
- [ ] AC8:工具页正确显示四类当前指向(agent-switch / 官方 / 第三方 / 未配置),且为只读检测。
- [ ] AC9:写入/检测失败时应用不崩溃,UI 显示可读错误。
- [ ] AC10:真实上游 API Key / OAuth token 绝不出现在被写入的工具配置文件中(只写占位符)。
- [ ] AC11:OpenCode 卡片仅展示手动配置说明,无接管开关。
- [ ] AC12:质量门通过——`cargo fmt --check`、`cargo check`(0 warning)、`cargo clippy --all-targets -- -D warnings`、`npm run build`。

## 暂不纳入范围

- `/claude-code/*`、`/codex/*` 的真实代理转发(子任务 5)。
- OpenCode 自动接管。
- 一键从备份恢复 / 关闭时自动还原原配置。
- Claude Code / Codex 之外的工具接管。
- 跨机器迁移接管配置(导入导出属子任务 8)。
