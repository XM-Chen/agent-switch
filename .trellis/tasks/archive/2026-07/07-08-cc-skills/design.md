# Design: Skills 管理（完整 ccs 范围）

## 范围与边界

- 按用户选择实现“完整 ccs”范围：多应用开关、SSOT 切换、GitHub/skills.sh 发现、导入/备份/恢复与批量更新。
- 实现仍采用 agent-switch 架构：Rust service + SQLite DAO + axum REST + React 页面，不搬 ccs Tauri command API。
- Skills 是外围管理独立 service：安装/导入/toggle/sync 后即时投影，不绑定 provider switch。

## 架构

```
前端 /skills
  ↓ REST
http/api/skills.rs
  ↓
services/skills
  ├─ storage: SSOT 路径、导入、hash、备份
  ├─ sync: symlink/copy/auto 投影到各 app skills 目录
  ├─ discovery: GitHub / skills.sh 搜索与安装
  └─ updates: 检查更新、单项/批量更新、失败回滚
  ↓
db/dao/skills.rs + skills / skill_repos tables
  ↓
SSOT(app_data/skills 或 ~/.agents/skills)
  ↓
~/.claude/skills / ~/.codex/skills / ~/.gemini/skills / OpenCode / Hermes
```

## 数据模型

- `skills`：一行代表一个已安装 skill，包含目录名、展示元数据、来源、hash、各 app enabled 标记、安装/更新时间。
- `skill_repos`：记录 repo/discovery 来源、分支、子目录、最近检查信息和更新状态。
- `app_metadata` 可保存 Skills 全局设置：SSOT 模式、sync 模式默认值、最后一次发现源配置。

## 路径与同步策略

- SSOT 默认使用 `app_data_dir()/skills`；可选迁移到 `~/.agents/skills`。
- live 目标目录由固定 app 映射表解析，严禁从请求体接受任意删除目标。
- `auto` 同步优先 symlink；失败 fallback copy，并在 API 结果中返回 warning。
- copy 同步使用临时目录 + rename；删除只删除托管项，且必须确认路径在允许的 live root 下。
- 目标同名非托管目录视为 conflict，不覆盖；用户可先 scan-unmanaged 导入纳管，或显式替换。

## 网络与发现

- GitHub/skills.sh 搜索、安装、检查更新均由用户显式触发。
- 下载后先写临时区，校验目录结构与入口文件，再进入 SSOT。
- 更新前创建备份；失败时恢复旧 SSOT 与 live 投影。

## 前端设计

- `/skills` 页面分区：已安装、发现/搜索、备份、同步状态。
- 已安装列表显示来源、hash/更新状态、各 app enabled 开关、投影状态和冲突提示。
- 所有 hard-to-reverse 操作（卸载、替换、批量更新、外部安装）需要确认弹窗。
- loading/error/empty/conflict 独立展示。

## 阶段化建议

1. 阶段 A：DB/DAO + SSOT + 本地目录/zip 导入 + Claude/Codex/Gemini/OpenCode/Hermes 投影基础。
2. 阶段 B：冲突扫描、备份/恢复、手动 sync/status。
3. 阶段 C：GitHub/skills.sh 发现、安装、检查更新、批量更新。
4. 阶段 D：完整前端打磨与跨平台路径测试。

## 风险与回滚

- 最高风险是目录删除/覆盖：所有删除必须路径白名单 + 托管标记 + conflict gate。
- Windows symlink 权限可能失败：auto fallback copy 是默认路径。
- 外部网络来源不可信：导入前预览、校验结构、用户确认。
- 回滚：DB 保留，关闭 UI 入口与 sync 调用即可停止 live 投影；已投影托管项可通过 sync disable 清理。