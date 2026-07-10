# Implement: Common Config 裸 JSON 编辑器

## Checklist

1. 梳理现有 `commonConfigApi`、ProviderForm meta/env helper，确认保存路径不覆盖 `meta.snapshot`。
2. 新增 Common Config 编辑器组件或 Settings 卡片：加载、编辑、校验 JSON object、保存、loading/error/empty 状态。
3. 在 Claude Code provider 表单加入 `common_config_enabled` 三态选择，保存时与 `meta.snapshot.env` 合并。
4. 增加前端测试：JSON 校验、三态序列化、meta 合并不丢 `snapshot.env`。
5. 增加/补后端测试（如缺口存在）：非 object PUT 拒绝、默认值、三态 meta 写入。
6. 回归验证：切换 provider 后 common deep-merge 生效、禁用 provider 不叠加、backfill 不吸收 common 键。

## Validation Commands

- `npm test -- --run`
- `npm run build`
- `cargo test`

## Review Gates

- grep 确认本任务不新增对 `~/.claude.json`、`CLAUDE.md`、`skills/`、`projects/` 的写入。
- 检查 provider 保存 payload：`meta.snapshot.env` 与 `common_config_enabled` 同时存在时不互相覆盖。
- 检查 UI 文案：明确“保存后下次切换或显式应用生效”。

## Rollback Points

- 前端入口与组件可独立回滚；后端 API 已存在，不需要回滚 schema。