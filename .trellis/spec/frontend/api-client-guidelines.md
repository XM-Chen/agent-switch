# 前端 API Client 规范

## 入口

所有管理 API 请求通过 `src/lib/api.ts` 发起。页面不直接拼 fetch URL。

## 错误处理

- 非 2xx 响应必须抛出包含状态码/错误消息的 Error。
- 页面不得在 error 状态静默把数据 fallback 成“空资源”并误导用户。

## 参数语义

- 日志筛选使用显式 `log_type=production|test`，不要复用 `tool=test` 表示测试日志。
- `log_type=production` 后端语义为排除 `tool='test'`；分页 total 与 items 必须在后端一致。
- 流式测试不得复用默认 `resp.json()` helper 解析 SSE。

## 测试

API 路径构造逻辑应提取为可测试函数，例如 `buildLogsPath`。

## API 方法保留原则

`lib/api.ts` 只保留**有真实页面或测试引用**的 API 方法。不要为了"完整 REST 表面"预置未调用的 `.get`/`.update` 等方法——它们会积累成死代码（本次精简一次性删掉了 7 个无调用者的 CRUD 方法）。

- 新增 API 方法时，同 PR 内必须有调用点（页面或测试）；否则不合并。
- 删除某个页面前，先确认其依赖的 API 方法是否还有其他引用；若无，一并删除。
- `logsApi.get` 这类"看似通用但实际只有一处用"的方法，保留前在注释里标明调用方，避免被误删。

## providersApi 契约

切换器页面 (`ProvidersPage`) 依赖的两个非标准 REST 语义：

- `switch(id)` → `POST /providers/{id}/switch`，返回 `{ warnings: string[] }`。`warnings` 非空表示切换成功但有非致命提示（如"备份跳过"），前端用 warning banner 展示；非 2xx 抛错走 error banner。**不要**把 warnings 当错误。
- `reorder(items)` → `POST /providers/reorder`，body `{ items: { id, sort_index }[] }`，`sort_index` 为 0 起连续重新编号后的新位置。前端用 `moveItem` 计算新顺序后整体提交，**不做乐观更新**（排序频次低，invalidate 列表即可）。
- `update(id, body)` 支持 `body.meta`，用于保存 Claude Code `meta.snapshot.env` 等 provider 元数据。`update` 只持久化 DB，不写 live；当前激活 provider 的 env 改动要另调 `switch(id)` 作为「应用到 live」。

### Claude Code env 写入示例

```ts
await providersApi.update(id, { meta: serializeClaudeEnv(oldMeta, switches) });
if (provider.is_current) await providersApi.switch(id);
```

## promptsApi 契约（仅保留页面真实调用）

Prompts 页面当前只需要列表、创建、更新、删除、启用/禁用、导入和 status：

- `list()` -> `GET /prompts`
- `create(body)` -> `POST /prompts`
- `update(id, body)` -> `PUT /prompts/{id}`
- `remove(id)` -> `DELETE /prompts/{id}`
- `enable(id)` / `disable(id)` -> 显式激活态操作，会触发 live `CLAUDE.md` 投影/清空
- `import()` -> `POST /prompts/import`
- `status()` -> `GET /prompts/status`

即使后端提供 `GET /api/prompts/{id}`，前端也不要预置 `promptsApi.get`，除非同 PR 内有真实页面/测试调用点；这条沿用上方“API 方法保留原则”。

## skillsApi 契约（外围管理，独立于切换）

Skills 页面通过 `skillsApi` 访问 `/skills`，所有联网方法仅在用户显式操作时调用：

- `list()` / `status()` → 列表与各 app 投影状态。
- `importDir(body)` / `importZip(body)` / `installRepo(body)` → 本地目录/zip/GitHub 安装，返回 `SkillImportReport`（含 sync 报告）。
- `setEnabled(id, app, enabled)` → 切换某 app 启用并即时投影，返回单 app `SkillSyncReport`。
- `sync()` → 全量重建投影。
- `uninstall(id)` → `DELETE /skills/{id}`，返回含 backup 时间戳的 `SkillUninstallReport`。
- `listBackups(directory?)` → 无 directory 时不带 query；有则 `?directory=<encoded>`。
- `restore(directory, timestamp)` / `checkUpdates()` / `update(ids?)` / `search(query)` / `scanUnmanaged()`。
- `update()` 缺省 ids 时发送 `{ ids: null }`（更新全部 GitHub 来源）；显式列表时 `{ ids: [...] }`。**不要**为「更新单个」单独造端点，复用 `update([id])`。

## sessionsApi 契约（只读浏览）

- `list(params)` / `messages(sourcePath)` → 只读，`app_type=claude-code` 必填；路径经后端 canonicalize 限定。前端不缓存消息详情（按需拉取）。

## deeplinkApi 契约（解析/预览 + 确认导入）

- `preview(url)` → 只解析不写库；`import(url)` → async，用户显式确认后调用。前端用 `redacted_url` 展示，不渲染原始含凭据 URL。skill 资源 preview 的 `blocked` 为 false（已接入安装），但仍需二次确认才 import。
