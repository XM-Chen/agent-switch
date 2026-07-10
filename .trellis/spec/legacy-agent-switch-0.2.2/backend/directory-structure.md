# Rust 后端目录结构

## 当前结构

```text
src-tauri/src/
├── lib.rs                 Tauri setup、服务启动、AppState 初始化
├── main.rs                桌面入口
├── app_state.rs           全局共享状态
├── config/paths.rs        应用数据目录与数据库路径
├── commands/              Tauri command
├── db/                    SQLite 连接、迁移、DAO
├── http/                  Axum 本地 HTTP 服务与 proxy
└── services/              业务服务层
```

## 分层边界

- `http/api/*`：解析 HTTP 请求、调用 DAO/service、返回 JSON；不要直接写复杂业务规则。
- `http/proxy/*`：本地代理、模型解析、认证注入、故障转移、SSE/stream guard。
- `services/*`：跨 handler 复用的业务能力，如 Codex OAuth、model sync、portability、translator。
- `db/dao/*`：数据库读写唯一入口；handler/service 不直接拼散落 SQL。
- `config/paths.rs`：不得回退到当前目录 `.`，避免数据库位置依赖启动 CWD。数据目录固定为 `dirs::data_dir()` 失败时回退 `std::env::temp_dir().join("agent-switch-data")`，再 `.join(APP_DIR_NAME)`；不要重新引入多级泛型 fallback helper。

## 新增模块规则

- 新 API 先放在 `http/api/<domain>.rs`，并在 `http/api/mod.rs`/router 中显式挂载。
- 新表必须通过 migrations 增量版本添加，并配套 DAO。
- 新协议转换优先在 `services/translator/` 中新增 translator pair，不把转换逻辑塞进 proxy 主循环。
- `http/api/mod.rs` 放跨 handler 共享的 helper（如 `encrypt_api_key`），不要在各 `api/<domain>.rs` 里重复实现同一 helper。

## 领域层边界

`services/provider/mod.rs` **只保留 `AppType` 枚举**（`as_str`/`from_str`/`all`/`config_dir`）。不要在此堆叠 `Provider`/`ProviderMeta`/`ProviderMode`/`NewProviderInput` 等"领域对象 + DAO 行转换"层——曾因投机预留而整体删除。Provider 的强类型视图由前端 / API handler 直接基于 `ProviderRow` 处理，`mode` 以字符串 (`"proxy"`/`"direct"`) 透传，需要强类型时在调用点就地匹配。新增领域类型前必须有真实调用点，否则不合并。
