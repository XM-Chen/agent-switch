# 审查技术设计 — 全代码库彻底审查

## 审查方法论

采用"多 finder 扇出 + 对抗式 verify + 综合 synthesize"三段式,而非单 agent 线性通读。理由:

- 代码库 ~17.4k LOC跨多子系统,单上下文难以同时持有 translator 流式细节、db 事务细节、前端 mutation 细节而不漂移。
- 独立 finder 各持一个视角,互不污染,覆盖面更广。
- 对抗式 verify(默认"误报",需被证伪为"真"才存活)剔除 LLM 审查常见的 plausible-but-wrong 发现。

## 工作流架构

```
Phase 1: Find    — N 个 finder 并行,每个持一个子系统/维度,产出 findings[]
Phase 2: Verify  — 每条 P0/P1 finding 派 ≥2 个独立 verifier 对抗验证,多数证伪则剔除
Phase 3: Synthesize — 1 个 synthesizer 去重、跨子系统关联、按严重度排序、生成报告
```

用 `pipeline(DIMENSIONS, find, verify)` 而非 barrier,使某子系统的 verify 与另一子系统的 find 并行,不浪费时间。

## 维度划分(Finder 清单)

后端(Rust):
1. `translator-streaming` — `anthropic_openai.rs` + `openai_responses.rs`:SSE 流式组装、tool_call index 映射、partial_json 转义、content block 序号、multi-tool-result 拆分。
2. `translator-native-helpers` — `native.rs` + `helpers.rs` + `mod.rs`:非流式翻译、共用辅助。
3. `proxy-core` — `http/proxy/mod.rs` + `stream_guard.rs` + `capability.rs` + `translate.rs`:请求转发、头过滤、流守卫、能力协商。
4. `proxy-failover-oauth` — `failover.rs` + `oauth_refresh.rs`:`should_failover` 错误分类、冷却、重试链、OAuth 刷新与凭据解密。
5. `db-layer` — `migrations.rs` + `dao/*`:事务边界、锁(upstream_model Mutex)、`busy_timeout`、迁移幂等、SQL 注入面。
6. `services-misc` — `codex_oauth.rs` + `model_sync.rs` + `tool_takeover/`:OAuth 流程、模型同步原子性、工具接管。
7. `portability` — `portability/{mod,apply,collect,crypto_box}.rs`:导入导出格式、加密盒、apply 的幂等/回滚。
8. `api-commands` — `http/api/*` + `commands/` + `app_state.rs` + `config/`:命令层、路由接线、状态管理、配置加载。

前端(React):
9. `frontend-data` — `lib/api.ts` + 各 `pages/*`:TanStack Query mutation/pending 守卫、错误处理、轮询、表单校验/重置。
10. `frontend-ui` — `components/*` + `pages/DashboardPage`(605 LOC 最大):空/加载/错误态、响应式、组件复用、死代码。

跨层(内置进相关 finder prompt,不单列维度):
- 前端 api.ts ↔ commands ↔ services/db 字段契约。
- translator 双向 SSE 端到端一致性。
- failover 错误分类 ↔ PRD"默认不切换"。

## 发现 Schema(JSON)

```json
{
  "id": "string",
  "title": "一句话",
  "severity": "P0|P1|P2|P3",
  "category": "correctness|security|data-loss|concurrency|resource-leak|quality",
  "file": "repo-relative path",
  "line": 1,
  "trigger": "触发/复现条件 — 输入或状态序列",
  "impact": "错误输出/崩溃/数据后果",
  "fix_direction": "建议修复方向(不实现)",
  "confidence": "high|medium"
}
```

## 严重度定义

- **P0**:数据损坏/丢失、凭据泄露、安全可利用、崩溃。
- **P1**:功能错误(错误输出、违反 PRD 契约如 failover 误切换)、资源泄漏。
- **P2**:边界条件缺陷、错误处理不完整、并发竞争(低触发概率)。
- **P3**:代码质量/简化/效率/死代码(进附录,不进主排序)。

## 对抗式验证协议

- 每条 P0/P1 finding 派 2 个独立 verifier,各被提示"默认此发现为误报,尝试证伪"。
- verifier 读实际代码确认触发路径真实存在,返回 `{verdict: real|refuted, reason}`。
- 2 票中 ≥1 票 `refuted` → 降级为 P2 重审或剔除;2 票均 `real` → 存活。
- P2/P3 不做对抗验证(finder 自报,synthesizer 去重即可)。

## 退化与覆盖标注

- 单个 finder 若因文件过大需采样,必须在返回中声明覆盖比例与未读部分。
- 报告"覆盖矩阵"列出每个子系统的 finder 状态、发现数、验证结果,使遗漏可见。

## 产出

`research/audit-report.md`:执行摘要 → P0/P1/P2 主表(按严重度) → P3 质量附录 → 已知限制节 → 覆盖矩阵 → 退化/采样声明。
