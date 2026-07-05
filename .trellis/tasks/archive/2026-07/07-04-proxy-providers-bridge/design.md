# Design: 代理模式与 providers 桥接及升级回填

## 架构与边界

本任务不引入新的运行期模块,只在启动序列加一个**一次性回填步骤** + 对应 DAO 函数 + 集成测试。

```
lib.rs setup():
  run_migrations()                 // v8 已加 mode/active_provider_id
  → providers::backfill_from_takeover(db)   // 新增:本任务核心
  → 构造 AppState / RouteProxy ...
```

边界:
- 回填是**纯 DB 操作**,不动 `tool_takeover` 状态、不动工具配置文件、不调 `tool_takeover::enable`。理由:`tool_takeover.enabled=1` 已表示工具正指向本地代理,回填只需在 `providers` 表里造出对应的 current 行,让 `/api/providers` 能列出、让 UI 能显示。再造接管是冗余且可能触发文件写。
- selector 管道完全不动。proxy provider 与上游选路解耦(已确认 `selector.rs` 不引用 providers/takeover)。

## 数据流

升级前(v7→v8)存量状态:
- `tool_takeover`: `{claude-code: enabled=1, mode='proxy'(v8 默认)}, {codex: enabled=1, ...}`
- `providers`: 空表

回填后:
- `providers`: 每个启用 tool 一行,`is_current=1`,`mode='proxy'`,`settings_config='{}'`
- `tool_takeover`: 不变

新装(无存量 takeover):
- `tool_takeover` 空 → 回填无操作,`providers` 保持空。用户通过 `/api/providers` POST 自行创建。

## 契约

### `providers::backfill_from_takeover(db) -> Result<BackfillReport, String>`

- 幂等:可重复调用。
- 对每个 tool(claude-code/codex):
  1. 查 `tool_takeover` 该 tool 行;`enabled=1` 才回填,否则跳过。
  2. 查 `providers` 该 app_type 是否已有 `is_current=1` 的行;有则**不覆盖**,跳过(尊重用户已配置的 current)。
  3. 无 current 时,造一行:
     - `id = format!("prov-backfill-{}", tool)`(确定性,幂等键)
     - `app_type = tool_to_app_type(tool)`(claude-code→claude-code,codex→codex)
     - `name = "默认代理 (claude-code)"` / `"默认代理 (codex)"`(中文名)
     - `mode = "proxy"`,`settings_config = "{}"`
     - `is_current = 1`,`sort_index = 0`
     - `meta = "{}"`
  4. 若 `id` 已存在(用户保留回填行但改了字段)→ 不覆盖。
- 返回 `BackfillReport { created: usize, skipped_existing_current: usize, skipped_takeover_disabled: usize }`。

### tool→app_type 映射
- 复用 `services::tool_takeover::Tool` 的 `as_str()` 与 `services::provider::AppType`。`claude-code`/`codex` 双向一致(已确认 `provider/mod.rs:23` 注释强调与 `tool_takeover::Tool` 保持一致)。

## 兼容性 / 迁移

- 回填在 v8 迁移**之后**运行,v8 已保证 `tool_takeover.mode` 列存在(默认 proxy)。回填不依赖 `mode` 值——它只看 `enabled`。
- 不新增迁移版本。回填是数据层一次性操作,不是 schema 变更。
- 已有 current provider 的用户(本任务之前手动建过)不被覆盖。
- 回填行用确定性 id `prov-backfill-<tool>`,用户后续可正常 PUT/DELETE/switch 它。

## 取舍

- **回填不调 `tool_takeover::enable`**:避免在启动期写用户工具配置文件(`~/.claude/settings.json`),降低风险;`tool_takeover` 已是 enabled 状态,无需重写。
- **回填行 `settings_config='{}'`**:proxy provider 不需要 endpoint 引用——上游由 selector 从 `endpoints` 表选路。这与子任务 3 switch(proxy)的行为一致(`enable` 走 `apply()` 写本地代理 + 占位符,不读 provider 的 settings_config)。
- **不做启动一致性校验**:见 PRD R2,只在回填点保证一致。

## 回滚

- 回填出错 → `Result<_, String>` 错误冒泡到 `lib.rs` setup,与迁移失败同等处理(panic 阻止启动)。
- 回滚点:若回填已写入部分行后失败,由于用确定性 id,下次启动重跑会跳过已存在 id,天然收敛。无需手动清理。
- 紧急回滚:删除 `providers` 表中 `id LIKE 'prov-backfill-%'` 的行即可恢复到回填前状态(`tool_takeover` 未被触碰)。
