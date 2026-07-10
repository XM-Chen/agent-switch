# IPC、API 与查询状态

## Tauri IPC 边界

前端统一从 `src/lib/api/<domain>.ts` 调用 `@tauri-apps/api/core.invoke`；例如 Provider CRUD/切换集中在 `providersApi`（`src/lib/api/providers.ts:48-95`），并通过 Tauri event 接收切换通知（`src/lib/api/providers.ts:124-130`）。

新增或修改 command 必须同时核对：

1. Rust command 函数及 serde 参数；
2. `src-tauri/src/lib.rs` 的 `generate_handler!` 注册；
3. TS API wrapper 的 command 名、camelCase 参数和返回类型；
4. query/mutation 的失效策略；
5. 成功、失败与事件测试。

组件不得直接写 `invoke("...")`，除应用启动等极少数集中入口外。已有散落调用在后续重构时逐步归入 API wrapper，不借裁剪任务做无关大改。

## TanStack Query 约定

全局默认行为见 `src/lib/query/queryClient.ts:1-13`：query 重试 1 次、窗口聚焦刷新、`staleTime=0`；mutation 不重试。

- 后端/磁盘/代理状态使用 Query；组件不维护第二份长期副本。
- mutation 成功后按领域 query key 精确 invalidate；批量切换或导入可失效整个领域根 key。
- 事件监听更新 cache 时要在卸载时调用 `UnlistenFn`。
- 需要乐观更新时必须有 rollback，Provider 切换默认以后端结果为准。
- API error 在 wrapper/hook 统一归一化，组件只展示可读中文错误。

## 状态分层

| 状态 | 存放位置 |
|---|---|
| Provider、MCP、Skills、Proxy、Usage 等业务状态 | SQLite/后端 + TanStack Query |
| 设备级 current provider、路径覆盖等 | ccs settings/Tauri Store；前端仅调用 API |
| 窗口内主题、视图等纯 UI 偏好 | localStorage/context（身份改造时审计 key） |
| 表单未提交草稿、dialog 开关 | 本地组件 state |
| Claude `settings.json` live 内容 | 后端 live adapter；前端不得直接文件读写 |

## 多应用裁剪

当前 API wrapper 广泛接受 `AppId`（如 `providersApi.getAll/switch/importDefault`，`src/lib/api/providers.ts:49-95`）。删除非 Claude 客户端时：

- 先保留 API 边界显式 `app: "claude"`，待 Rust 单应用化稳定后再决定是否删除参数；
- 不把 Claude Provider 上游类型误当成 AppId；
- 清理对应 query keys、事件 payload 分支与缓存，不允许幽灵 app 数据残留；
- 对每个删掉的 invoke command 做前端引用扫描与 Rust handler 注册扫描。

## 非 loopback 代理风险确认（目标要求）

ccs 当前代理默认 `127.0.0.1`，但设置可改监听地址。目标实现中，前端保存非 `127.0.0.1`/`localhost`/`::1` 地址之前必须：

1. 展示局域网暴露与本地 token 鉴权的中文风险；
2. 要求显式确认；
3. 将确认状态持久化而非只存在本次 dialog；
4. 后端先具备全路由鉴权中间件，前端提示不能代替安全控制。

## 同步风险确认（目标要求）

首次启用 WebDAV/S3 前必须明确披露：`db.sql` 未做客户端内容加密，可能含 Provider API token，远端管理员或凭据持有者可读取。必须持久化显式确认，不能只写“网络传输有风险”。
