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
- `config/paths.rs`：不得回退到当前目录 `.`，避免数据库位置依赖启动 CWD。

## 新增模块规则

- 新 API 先放在 `http/api/<domain>.rs`，并在 `http/api/mod.rs`/router 中显式挂载。
- 新表必须通过 migrations 增量版本添加，并配套 DAO。
- 新协议转换优先在 `services/translator/` 中新增 translator pair，不把转换逻辑塞进 proxy 主循环。
