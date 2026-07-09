# Design: Common Config 裸 JSON 编辑器

## 范围与边界

- 仅补 UI 与前端状态编排；后端 Common Config API、provider `meta.common_config_enabled` 三态、地基切换语义已存在。
- 只支持 Claude Code：tool 固定为 `claude-code`，非 Claude provider 不展示三态开关。
- 生效方式保持地基语义：保存 DB 后不隐式改 live；切换 provider 或显式应用时才落 `~/.claude/settings.json`。

## 架构

```
Settings/Provider UI
  ├─ commonConfigApi.get('claude-code') / put('claude-code', object)
  └─ providersApi.update(id, { common_config_enabled })
        ↓
app_metadata.common_config_claude-code
providers.meta.common_config_enabled
        ↓ 切换时
write_claude_snapshot_layer(snapshot + common)
        ↓
~/.claude/settings.json
```

## 前端落点

- `src/lib/api.ts`：复用已存在 `commonConfigApi` 与 `UpdateProviderBody.common_config_enabled`。
- `src/components/providers/ProviderForm.tsx`：增加 Claude Code provider 的三态选择器，保存时与现有 `meta.snapshot.env` 合并提交。
- 新增或内联 Common Config 编辑器组件：读取当前 JSON、用 textarea/monospace 编辑、保存前 `JSON.parse` + object 校验。
- 入口建议放在 Provider 表单 Claude Code 区块，或 Settings 页新增 Claude Code Common Config 卡片；二者共用同一编辑器状态逻辑。

## 数据合同

- Common Config 必须是 JSON object；后端已拒绝非 object，前端提前给出错误。
- `common_config_enabled` 三态：`null` = 清除显式设置、跟随默认；`true` = 强制启用；`false` = 强制禁用。
- 保存 provider 时，若同时提交 `meta` 和 `common_config_enabled`，后端会以 `meta` 为基底叠加开关；前端仍要避免丢弃现有 `meta`。

## 兼容与回归

- 不改 DB schema、不改 tool_takeover 核心、不改 direct/proxy 连接层。
- Common Config 可包含 `env`，但连接层 env 仍由 `apply` / `apply_direct` 最后注入；后续验证要覆盖 token 不落 DB。
- Codex 路径不展示、不提交 Common Config 开关。

## 风险与回滚

- 最大风险是前端保存 provider 时覆盖 `meta.snapshot`；通过 helper 合并与测试覆盖。
- JSON 裸编辑器会允许用户写任意键，错误配置可能影响 Claude Code；UI 需要提示“保存后下次切换或显式应用生效”。
- 回滚只需移除前端入口；后端已有 API 可保留。