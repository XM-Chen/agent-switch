# Portability 导入导出规范

## 导出模式

- `full_backup`：本机完整备份，使用系统主密钥解密/加密敏感凭据，主要用于同机恢复。
- `portable`：可迁移脱敏配置，使用用户密码派生密钥，不包含 API key/OAuth token。

## mode / kdf 绑定

导入前必须交叉校验：

| mode | kdf | 语义 |
|------|-----|------|
| full_backup | none/master-key | 允许，使用系统主密钥 |
| portable | argon2id | 允许，使用用户密码 |
| full_backup + argon2id | 拒绝 |
| portable + none/master-key | 拒绝 |

畸形组合必须在解密前返回明确错误。

## replace 语义

full backup replace 必须覆盖完整状态：

- 清空并恢复 accounts/endpoints/endpoint_models/model_aliases/route_settings。
- 恢复 ui_settings 白名单键；包中缺失的白名单键按“未设置”处理。
- tool takeover 状态导入后必须保持 disabled，不自动写入外部工具配置。

## merge 语义

portable merge 遇到孤儿 endpoint_model / alias 不得静默跳过或写入空外键；必须在 ImportReport 中提供 warnings/skipped 计数。

## 文件名

自动 DB 备份文件名必须 Windows-safe，建议 `YYYYMMDD-HHMMSSZ`，不得含 `:`、路径分隔符、小数秒点或空格。

## 密码提示

弱密码检测按字符数而非字节数计算，避免 CJK 密码误判。
