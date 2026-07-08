# 实现计划：env 行为开关注入（cc-env-switches，仅 Claude Code）

> 依据 `design.md`。决策 C = 复用 `meta.snapshot.env`，**不新增后端 API/迁移/service**。主体是前端结构化编辑器 + 预设模板，后端仅在「应用到 live」时复用 `switch_claude(prev=target)`。

## 验证命令

| 范围 | 命令 |
|---|---|
| Rust 编译 | `cargo build --manifest-path src-tauri/Cargo.toml` |
| Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml --lib` |
| 格式+lint | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` |
| 前端 | `npm run build && npm test` |

## 有序实现清单

### 阶段 0：前端预设模板 + env 解析 helper ✅
- [ ] `src/config/claudeProviderPresets.ts`：首批预设（GLM/Kimi/MiniMax 等对齐后端示例集 + Bedrock 预设），每个 = `{ name, env: {...} }`，不含连接层。Bedrock 预设含 `CLAUDE_CODE_USE_BEDROCK`/`AWS_REGION`/`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`（明文，用户拍板）。
- [ ] `src/components/providers/` 下新增 env 解析/序列化 helper（参考 ccs `useModelState.parseModelsFromConfig` / `handleModelChange`）：`meta.snapshot.env` ↔ 结构化字段双向转换；`[1M]` 标记解析（`stripClaudeOneMMarker`/`setClaudeOneMMarker`）。
- [ ] 单测（vitest）：env 解析/序列化往返、1M 标记、空值删键。

### 阶段 1：结构化 env 编辑器 UI ✅
- [ ] `ProviderForm.tsx` 新增「Claude Code 行为开关」分区（仅 `app_type=claude-code`）：模型三档（+`_NAME`+1M 勾选）+ 兜底模型 + `API_TIMEOUT_MS` + Bedrock 开关（`CLAUDE_CODE_USE_BEDROCK`+`AWS_REGION`+`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` 明文，secret 用密码框遮显）+ 常见 `CLAUDE_CODE_*` + 裸 JSON 逃生舱 + 预设选择。
- [ ] 读写 `meta.snapshot.env`（经既有 `meta` 透传，提交走 `PUT /api/providers/{id}`）。
- [ ] 状态同步：结构化字段 ↔ 裸 JSON 双向（`isUserEditingRef` 防回填覆盖，参考 ccs）。
- [ ] 校验：模型 id 非空接受、`API_TIMEOUT_MS` 数字、Bedrock 布尔；空值删键。
- [ ] 禁止暴露 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`。

### 阶段 2：「应用到 live」按钮 ✅
- [ ] 仅当前激活 provider 显示「应用到 live」按钮 → 调既有切换链路 `switch_claude(prev=target)` 重切，把更新后的 `meta.snapshot.env` 落 live（复用地基三层）。
- [ ] 非激活 provider 不显示（编辑保存后下次切换自然生效）。
- [ ] 单测/集成：编辑 env → 应用 → live `settings.json.env` 含新键 + 连接层仍正确。

### 阶段 3：回归验证 ✅
- [ ] 后端无改动确认：grep 确认未新增对 `settings.json` 的直接写入（仍只走 `switch_claude`/`apply`/`apply_direct`）；未新增 DB 迁移；Codex 路径零改动。
- [ ] 回归测试：编辑 env → 切走 → 切回 → env 开关如实恢复（地基 backfill 往返）；direct provider 编辑后 token 明文不落 DB；proxy `is_bedrock_provider` 识别 Bedrock 预设一致。
- [ ] `cargo test` + `npm test` 全绿。

## 风险文件 / 回滚点

- `ProviderForm.tsx` + 新增 env helper/预设模块：纯前端，回滚 = 撤分区 + 删模块，无后端副作用。
- `meta.snapshot.env` 写入：经既有 provider update（meta 透传），不新增写链路；与地基 backfill 交互需回归测试。
- 「应用到 live」走 `switch_claude(prev=target)`：复用地基链路，不新建重切路径；reapply 保持现状不动。
- AWS 敏感凭证（用户拍板明文纳入）：`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` 明文落 `meta.snapshot`（DB）+ 明文落 live。已确认 portability `collect` 不导出 `providers` 表 → `meta.snapshot` 不经导出泄漏（暴露面仅 DB 文件 + live settings.json，属明文决策预期）。UI secret 框遮显。

## 完成前检查

- [ ] 四条验证命令全绿。
- [ ] AC1-AC8 逐项覆盖（结构化写入落 live / 切换往返不丢 / 预设预填 / 逃生舱 / Bedrock 预设含凭证 / providers 不经 portability 导出 / 不干扰连接加密 / 回归）。
- [ ] grep 确认无新增对 `settings.json` 的直接写入、无新 DB 迁移、Codex 路径无改动。
