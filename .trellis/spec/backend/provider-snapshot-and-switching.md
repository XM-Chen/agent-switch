# Provider 全文快照与切换

## 数据模型

`Provider.settings_config: serde_json::Value`（`src-tauri/src/provider.rs:13-15`）直接保存 Claude 用户级 `~/.claude/settings.json` 的完整 JSON 快照。它是唯一快照 SSOT：

- 不新增 `meta.snapshot`；
- 不定义只含 `env.ANTHROPIC_*` 的固定 schema；
- hooks、permissions、statusLine、sandbox、插件和未来未知字段必须往返保留；
- `meta` 只保存内部 Provider 元数据，不保存第二份设置快照。

## 三层 live 语义

```text
DB provider.settings_config（供应商完整快照）
  + 启用的 Common Config（全局共享层，深合并）
  → Claude sanitizer（仅剥内部字段）
  → ~/.claude/settings.json（live，整体原子写入）
```

### 切入目标 Provider

1. 从 DB 取目标 Provider；
2. `build_effective_settings_with_common_config` 克隆 `settings_config` 并按配置深合并 Common Config（`src-tauri/src/services/provider/live.rs:483-506`）；
3. `sanitize_claude_settings_for_live` 只删除内部字段 `api_format/apiFormat/openrouter_compat_mode/openrouterCompatMode`（`live.rs:23-33`）；
4. `write_live_snapshot` 写 live（`live.rs:739`），底层 `write_json_file` / `atomic_write` 集中处理（`src-tauri/src/config.rs:274-319`）。

未知字段不得被 sanitizer、表单或序列化重建误删。

### 切出当前 Provider

`switch_normal` 的标准流程（`src-tauri/src/services/provider/mod.rs:2159-2244`）：

1. 获取有效 current provider；
2. 切到不同目标前读取当前 live；
3. 将 live 中共享变更同步到 Common Config；
4. `strip_common_config_from_live_settings` 深剥离 Common Config，避免共享层被重复吸收；
5. 将剩余完整 JSON 回填当前 Provider 的 `settings_config` 并保存；
6. 更新 device-level/current DB；
7. 把目标有效快照写 live。

ccs 目前 backfill 失败只加入 warning 并继续；Agent Switch 首启保护和首次切换需进一步保证：无法保全现有 live 时明确提示并阻止静默覆盖。

## Common Config

- 深合并对象；目标 provider 保有非共享字段（`live.rs:98-113`）。
- 深剥离共享对象/数组时只移除匹配子集，不能删用户其他项目（`live.rs:50-129,371`）。
- Provider 是否使用 Common Config 是 Provider 元数据/配置，不因 snippet 存在就强制启用。
- 回填失败保留 live 原值并产生 warning，不把空对象保存为 Provider 快照。

## 代理接管模式

proxy takeover 时 live 由代理占位配置拥有，Provider 切换走 hot switch，不按普通模式回填占位内容（`src-tauri/src/services/provider/mod.rs:2140-2155`）。禁止把代理的 localhost URL/token 占位快照吸收到实际 Provider。

## 首启 import-before-seed

目标数据根为空时：

- 若 `~/.claude/settings.json` 存在、可解析、未被代理占位接管，全文导入为 `default` 并设 current；
- 然后 seed 精选官方模板；
- live 不存在才直接 seed；
- 无效 JSON、读取失败、代理占位不允许静默覆盖；
- 不读取 `~/.cc-switch` 或旧 Agent Switch DB。

## 必测场景

- 未知顶层对象、嵌套 hooks/permissions、数组、`env` 非连接键往返；
- 表单修改已知字段后未知字段仍在；
- Common Config 合并与剥离不重复、不越删；
- 切出回填外部编辑的字段；
- takeover 模式不回填占位；
- live 写失败时 current 状态与 DB 不产生不可恢复分叉；
- 新 HOME 首启 import-before-seed，诱饵 `~/.cc-switch` 保持字节不变。
