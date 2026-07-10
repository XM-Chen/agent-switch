# Design: 切换语义增强（回填保护 + Common Config Snippet，A1-hybrid + B1）

## 范围决策：Codex 连带（2026-07-07 用户拍板）

- **优先保证 Claude Code 功能完善，Codex 保持现状不动**。本子任务只给 **Claude Code** 加全文快照层；**Codex** 完全不改。
- 实现层面：新增的快照/backfill/common config 逻辑**只在 Claude Code 分支触发**；Codex 的 `enable`/`enable_direct`/`disable`/`reapply` 路径逐字节不变。
- 好处：Codex 零改动、零回归；共享的 `DirectSettings` / `resolve_direct_config` / `apply_direct` 全部保持原样。

## 关键设计修正（2026-07-07，实现期重读代码后）

**不重定义 `settings_config` 语义，改用 `meta.snapshot` 承载全文快照。**

原 design 想把 `settings_config` 从「端点引用」改成「全文快照」。重读调用图后发现该方案 blast radius 过大且违背既有 spec：
- `settings_config` 被 `resolve_direct_config`（Claude+Codex 共用）、tools-toggle、`reapply`、ccs 导入器同时消费为端点引用；重定义它需要 DB 迁移 + 改所有共享消费者 + 冒险破坏 Codex 与 192 个既有测试。
- `database-guidelines.md` 明确：providers 表按原样存 JSON，direct 凭证加解密职责在接管服务侧。

**新方案**：`settings_config` **保持不变**（仍是连接规格：proxy 或 `{endpoint_id, model, ...}`）。per-provider 全文快照存到 **`meta.snapshot`**（provider.meta 已是 JSON TEXT，无需 schema 迁移）。

优势：
- **无 `settings_config` 迁移** → Codex / 存量 provider / 既有测试零风险。
- **token 天然不落库**：快照存入 `meta.snapshot` 前先删掉连接层 env（`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`）。真实 token 只由既有 `apply_direct` 在写 live 时注入，snapshot 里根本没有 token 字段 → 无需 `${ENDPOINT}` 占位符、无需 resolve/redact 解密逻辑，加密约束自动满足。
- **复用既有已测代码**：连接层仍由 `claude_code::apply`（proxy）/ `apply_direct`（direct）注入，这两个函数及其全部测试不动。

## 架构总览

写 `~/.claude/settings.json` 分两步（顺序关键）：

```
第 1 步：写非连接层（快照 + common config）
    effective_base = deep_merge(provider.meta.snapshot, common_config?)   // build_effective
    write_live_snapshot(config_dir, effective_base)                       // 整文件覆盖 + 原子写

第 2 步：注入连接层（复用既有 apply / apply_direct，读-改-写合并）
    proxy  → claude_code::apply(config_dir)          // 写本地代理 URL + agent-switch-managed 占位符
    direct → claude_code::apply_direct(config_dir, cfg)  // 写真实 base_url + 解密明文 token（+ model）
```

第 2 步的 `apply`/`apply_direct` 是「读 live → 改 env 连接键 → 写回」，正好把连接层叠加到第 1 步写出的快照之上。二者天然协作，无需改这两个函数。

## 切换流程（`perform_switch` 内，设 is_current 之后）

```
1. backfill 切走前 provider（仅当 prev 是 Claude Code）:
   live = read_live(~/.claude/settings.json)
   snapshot = strip_connection_env(live)          // 删 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN
   snapshot = deep_remove(snapshot, common?)       // strip common config 贡献的键
   prev.meta.snapshot = snapshot                   // 存回「切走前」provider（无 token）
2. 接管目标 provider（既有逻辑 + 快照前置）:
   写 target.meta.snapshot ⊕ common → live         // 第 1 步
   enable / enable_direct                          // 第 2 步（既有连接注入）
```

关键：backfill 存的 snapshot **不含连接 env**，因此 DB 永无明文 token；连接 env 每次切换由 mode + endpoint 重新推导。

## 数据模型

- `providers.settings_config`：**不变**（proxy = `{}` 或代理配置；direct = `{endpoint_id, model, wire_api, requires_openai_auth}`）。
- `providers.meta.snapshot`：**新增**，per-provider 的 `settings.json` 非连接键全文快照（hooks/permissions/statusLine/env 内非连接键/...），**不含** `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN`。缺省无此键 → 视为空快照 `{}`。
- `providers.meta.common_config_enabled`：三态（缺省=跟随默认 true / 显式 true / 显式 false）。
- `app_metadata` 表键 `common_config_claude-code`：全局 common config JSON 文本，默认 `{"includeCoAuthoredBy": false}`（复用既有 kv 表，**无需新表/新迁移**）。

## 关键算法（已实现于 `json_merge.rs` + `claude_snapshot.rs`）

- `deep_merge(base, source)`：递归合并，source 覆盖（common 叠加用）。✅ 已实现+测试
- `deep_remove(target, subset)`：strip common config 键（backfill 用）。✅ 已实现+测试
- `is_subset(subset, target)`：legacy 三态子集检测。✅ 已实现+测试
- `build_effective(snapshot, common, enabled)`：组装 deep_merge。✅ 已实现+测试
- `write_live_snapshot` / `read_live`：整文件覆盖（serde 无 preserve_order → BTreeMap 天然排序键）+ 原子写。✅ 已实现+测试
- **待补**：`strip_connection_env(settings)` —— 删除 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`，用于 backfill 存快照前剥离连接层（替代原 `redact_token` 占位符方案；更简单、更安全，因为直接删除而非脱敏）。

> 注：`claude_snapshot.rs` 里的 `${ENDPOINT}` 占位符 / `redact_token` / `apply_connection` / `ClaudeConnection` 在新方案下**不再需要**（连接层复用 `apply`/`apply_direct`，快照不含 token）。将在接线阶段清理为 `strip_connection_env` + `build_effective` + snapshot 读写这几个真正用到的函数。

## 三态 common_config_enabled（对齐 ccs live.rs:354-369）

- 显式 `true` → 叠加 common。
- 显式 `false` → 不叠加。
- 缺省（None）→ 默认叠加（新 provider 默认吃 common）。legacy 子集检测（`is_subset`）作为可选增强，本批可先用「缺省=true」简化。

## Proxy vs Direct 统一

- 两模式第 1 步相同（写快照 ⊕ common）。
- 第 2 步分流：proxy → `apply`（占位符）；direct → `apply_direct`（解密真实 token）。
- direct 的 base_url/token/model 仍来自 `settings_config` 的 endpoint 引用 → `resolve_direct_config`（**不变**）。

## 行为变化（需验收显式测试）

现状「用户 hooks 全局共享、跨 provider 不变、永不丢」→ 新行为「per-provider：切走前 backfill 捕获进 `meta.snapshot`，切回还原」。这是 A1 的预期语义变化。

**升级兼容**：存量 provider 无 `meta.snapshot`（视为空快照）。首次切走时 backfill 会把当前 live 的非连接键捕获进快照，此后即 per-provider。**风险**：升级后首次切换某 provider 前，若从未切走过它，其快照为空 → 第 1 步写空 live 再叠加连接层，会丢失用户 live 里已有的 hooks。**缓解**：切到目标时，若目标 `meta.snapshot` 缺失，第 1 步跳过整文件覆盖，退回既有 merge 语义（只由 `apply`/`apply_direct` 改连接键，保留 live 其它键）——即「快照缺失 = 老行为，快照存在 = 新行为」，平滑过渡。

## 灾难恢复

- 保留既有 `backup_before_write` 文件级 `.bak`（首次接管时）。
- backfill 本身即防丢机制（切走前捕获）。整文件覆盖仅在目标快照存在时发生，且切走前已 backfill，无净丢失。

## B1 解耦边界

- 本子任务**不**碰 `~/.claude.json` / `CLAUDE.md` / `skills/`。
- 预留切换成功后的 sync 钩子点（在 `perform_switch` 成功返回前留 `on_switch_succeeded` 扩展位，供 cc-mcp/cc-prompts/cc-skills 挂 sync）。本批不实现 sync。

## 兼容 / 迁移

- **无 DB schema 迁移**（复用 `meta` JSON + `app_metadata` kv）。
- `services/importers/ccs.rs`：不变。ccs 导入仍建 endpoint + direct provider（`settings_config` = endpoint 引用）。ccs 的 settings.json 全文里的非连接键（hooks 等）可选地导入 `meta.snapshot`（本批可延后，作为增强）。

## 风险 / 回滚点

- **最高风险降级**：不再动 `settings_config` 语义 → 无破坏性迁移。主要风险变为 `perform_switch` 切换链路重写（影响所有 Claude 切换）+ 升级首切快照缺失的兼容处理（已由「快照缺失=老行为」缓解）。
- 切换链路重写必须单测覆盖：proxy↔proxy、proxy↔direct、direct↔direct 往返；hooks per-provider 隔离 + 切回还原；common config 叠加/关闭；**backfill 后 `meta.snapshot` 绝无 token**（安全断言）。
- Codex 回归：跑既有 tool_takeover 全部测试确认 Codex 路径零变化。
