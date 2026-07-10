# 项目指南索引

## 基线与目标

- 源基线：cc-switch v3.16.5，commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93`。
- 当前分支：`agent-switch-ccs`。
- 目标：Agent Switch 0.3.0，仅 Windows、简体中文、Claude Code；基于 ccs 成熟架构分阶段裁剪。
- 父任务权威来源：`.trellis/tasks/07-10-ccs-baseline-migration/`。

## 指南

1. [项目与工具链约定](project-and-toolchain.md)
2. [单应用跨层裁剪](single-app-trimming.md)
3. [产品身份、数据目录与更新](identity-data-and-updater.md)
4. [变更验证矩阵](change-validation-matrix.md)

## 触发规则

| 修改范围 | 必读 |
|---|---|
| AppId/AppType、Provider 切换、MCP、Skills、Sessions、Deep Link | `single-app-trimming.md` + 前后端索引 |
| Provider 表单、settings.json、Common Config、首启导入 | `../backend/provider-snapshot-and-switching.md` |
| Proxy、格式转换、OAuth/Copilot、监听地址 | `../backend/proxy-security-and-managed-upstreams.md` |
| DB、备份、WebDAV/S3 | `../backend/database-backup-and-sync.md` |
| 名称、版本、路径、Deep Link、WiX、updater、release | `identity-data-and-updater.md` |
| 任意代码修改 | `change-validation-matrix.md` |

## 旧规范

`../legacy-agent-switch-0.2.2/` 是旧 `main` 产品规范归档，不是当前实施规范。首期“显式移植旧 Agent Switch 能力 = 无”；如需恢复旧能力，另建需求和设计任务。

## 文档语言

active spec、用户文档、任务 PRD/设计/实施计划使用中文。代码标识与既有英文注释遵循周边风格，不为翻译而制造大 diff。
