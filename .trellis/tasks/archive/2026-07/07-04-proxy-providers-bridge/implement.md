# Implement Plan: 代理模式与 providers 桥接及升级回填

## 执行清单(按序)

### 1. DAO 层:providers::backfill_from_takeover
- [ ] 在 `src/db/dao/providers.rs` 加 `backfill_from_takeover(db) -> Result<BackfillReport, String>`。
  - 内部按 tool 迭代;用 `dao::tool_takeover::get_state` 查 enabled;用 `providers::get_current` 查是否已有 current;用 `providers::create` 造行(NewProvider 字段按 design.md 契约)。
  - `BackfillReport { created, skipped_existing_current, skipped_takeover_disabled }` derive `Debug, PartialEq, Eq`。
- [ ] tool→app_type 映射复用现有常量,不硬编码新字符串(参考 `provider/mod.rs` 注释)。
- [ ] 单测(在 providers.rs tests 模块):
  - 空 takeover → 无操作,report 全 0。
  - takeover enabled=1 → 造行,is_current=1,mode=proxy,settings_config="{}"。
  - 已有 current → 不覆盖,skipped_existing_current 计数。
  - 幂等:连跑两次第二次 created=0。
  - takeover enabled=0 → 跳过。

### 2. 启动接线:lib.rs
- [ ] 在 `run_migrations` 之后、`AppState` 构造之前调 `providers::backfill_from_takeover(db.as_ref())`。
- [ ] 失败按迁移失败同等处理(`panic!` 阻止启动);成功 `tracing::info!` 打 report。
- [ ] 不动 AppState 字段、不动 RouteProxy 构造。

### 3. 集成测试:回填后转发行为
- [ ] 在 `http/proxy/integration_tests.rs` 或新 test 模块加:
  - 构造 `tool_takeover.enabled=1` + 空 providers → 调回填 → `providers.is_current=1`。
  - 经 `RouteProxy` 转发一个请求,断言走本地代理 + selector 从 `endpoints` 选路上游(行为与无 providers 时一致)。
  - 复用 integration_tests.rs 的 helper 模式造内存 DB + 端点。

### 4. 门禁
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --lib`(全量,确认回填 + 既有 158 测试不回归)

## 风险文件 / 回滚点

- `src/db/dao/providers.rs` — 新增函数,改动隔离,低风险。
- `src/lib.rs` — 启动序列加一行调用,改动极小,但位置敏感(必须在迁移后、AppState 前)。回滚:删掉调用即可,无 schema 残留。
- 回填数据回滚:`DELETE FROM providers WHERE id LIKE 'prov-backfill-%'`。

## review 门

- DAO 单测全过后才接线 lib.rs。
- 接线后跑全量门禁,158 + 新增测试全绿才算完成。
- 集成测试必须真实走 RouteProxy 转发路径,不能只测 DB 状态。

## 依赖与前置

- 依赖子任务 1(providers DAO/schema)、2(tool_takeover v8 列)、3(switch API)——均已合并。
- 不阻塞子任务 5(前端),但前端会受益于回填后的非空 provider 列表。
