# 审计方法论与技术设计

## 审计目标

把用户对 agent-switch 的原始需求与当前项目实现逐项对照，判断对齐 / 部分对齐 / 未对齐 / 超额完成 / 无法验证，并区分超额完成属于合理演进还是潜在过度工程。

## 证据来源与权重

1. **任务文档（最高权重，需求源）**：`.trellis/tasks/archive/2026-06/06-26-agent-switch-web-router-mvp/prd.md` 父任务 + 8 个子任务 PRD + Dashboard 子任务 PRD。用户需求、约束、验收标准、排除项以这些文件为准。
2. **历史会话（补充权重，澄清用户口径）**：`trellis mem search` / `trellis mem extract`，用于确认用户在对话中的原始口径。OpenCode 平台历史不可索引，需在结论中标注。
3. **当前代码（实现证据）**：`src-tauri/src/**`（Rust 后端）、`src/**`（前端）、`src-tauri/migrations` 或 `db/migrations.rs`、`package.json`、`Cargo.toml`、`tauri.conf.json`。
4. **提交历史（交付证据）**：`git log --oneline`、归档任务状态。
5. **非破坏性验证命令（交付可靠性证据）**：`npm run build`、`cargo check`、`cargo fmt --check`、`cargo clippy`。

## 需求域划分（审计矩阵骨架）

把需求归并成以下域，逐域判定状态：

| 域 | 对应子任务 | 关键验收锚点 |
|---|---|---|
| D1 应用骨架与本地服务 | app-shell | Tauri+Rust+Web、单进程单端口 `127.0.0.1:42567`、路径隔离、`/health`、SQLite 迁移、端口占用不自动换 |
| D2 账号端点与凭据安全 | accounts-endpoints | accounts/endpoints 表、AES-GCM + Keychain、Codex OAuth PKCE + 临时回调端口 1455、脱敏显示 |
| D3 模型管理刷新与别名 | model-management | endpoint_models/model_aliases 表、手动/启动/定时刷新、6h+jitter、host/凭据限流、能力类型、别名解析优先级、失效提示 |
| D4 工具接管 | tool-takeover | Claude Code/Codex 开关、写入+备份、当前指向四类检测、OpenCode 仅手动、关闭不自动还原 |
| D5 路由与故障转移核心 | routing-failover | `/claude-code` `/codex` 真实转发、Native Passthrough + 转换器、Fill-First/Round-Robin、错误分类、冷却、模型级锁、OAuth token 自动刷新、fallback 链路日志、模型角色映射剥离 `[1M]` |
| D6 OpenAI-compatible v1 多端点 | v1-endpoints | `/v1/chat/completions` `/v1/responses` `/v1/embeddings` `/v1/images` `/v1/audio` `/v1/models`、能力双重过滤、images/audio 透明流转 |
| D7 真实链路测试与调试器 | chain-testing | `/api/tests`、test_only 不写冷却、流式调试器、Images/Audio blob 临时展示、取消按钮、token 消耗提示 |
| D8 导入导出与设置 | import-export | 完整备份（主密钥）/脱敏（Argon2id 密码）、冲突双模式分治、导入事务、自动接管导入后强制关闭、风险提示中文 |
| D9 Dashboard 总览页 | dashboard | 4 类计数 + 工具接管 + 自动刷新 + 近 10 条日志 + 端点健康、纯前端组合、空状态引导 |
| D0 跨模块约束 | 父任务 | 中文 UI/文档、固定服务地址、不做云同步、不做托盘常驻、本地服务不认证仅绑 127.0.0.1、凭据不外泄日志 |

## 状态判定定义

- **已对齐**：需求有实现，且有证据（代码/命令/任务归档）支撑达标。
- **部分对齐**：核心已实现但存在缺口（如某子能力缺失、UI 占位、未跑通质量门）。
- **未对齐**：需求明确要求但仓库中找不到对应实现，或实现与需求语义冲突。
- **超额完成**：实现了父/子任务 PRD 未要求、用户未明确要求的能力。
- **无法验证**：需要真实上游、真实工具配置或 OpenCode 历史日志才能验证，本次审计不能安全执行。

## 超额完成性质判定

- **合理超额**：补齐了 PRD 中“暂不纳入”但实际是闭环必需的粘合代码（如错误处理、空状态、配置校验）。
- **可能过度工程**：超出单机本地工具定位的复杂度（如重型聚合、企业级多规则引擎、云同步预留实现而非预留点）。
- **需产品确认**：方向合理但范围超出用户原始口径，应回交用户确认是否保留。

## 非破坏性验证范围

允许：

- `npm run build`
- `cargo check`（workspace 或 `src-tauri`）
- `cargo fmt --check`
- `cargo clippy --all-targets`（若耗时可控）
- 只读命令：`git log`、`git show`、目录/文件读取、`trellis mem`

禁止：

- 任何写入本机 Claude Code / Codex / OpenCode 配置的命令。
- 任何向真实上游发送请求的命令（curl 真实 API、运行会触发真实转发的测试）。
- 修改业务代码或配置文件。
- 安装新依赖或改变环境。