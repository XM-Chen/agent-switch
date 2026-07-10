# 执行计划 — 导入导出与设置

> 配套 `prd.md` / `design.md`。按依赖顺序:依赖 → 容器/加密 → 收集/应用 → API → 前端 → 质量门 → 运行验证。

## 实现顺序

### 1. 依赖

- [ ] `src-tauri/Cargo.toml` 增 `argon2 = "0.5"`、`flate2 = "1"`。

### 2. 服务层 `services/portability/`

- [ ] `package.rs`:`ExportPackage`、`KdfParams`、`Payload` 及各 `*Export` 结构;序列化/反序列化;`format_version=1` 校验。
- [ ] `crypto_box.rs`:
  - 主密钥模式 seal/open(复用 keychain 主密钥 + AES-GCM,包级 AAD)。
  - 密码模式 seal/open(Argon2id 派生 → AES-GCM);弱密码检测。
  - gzip 压缩/解压(flate2)。
- [ ] `collect.rs`:从各表收集 → `Payload`;`portable` 模式剔除凭据列与 request_logs。
- [ ] `apply.rs`:
  - replace 策略(full_backup,保留 id,事务内 DELETE+INSERT)。
  - merge 策略(portable,新 UUID + id 重映射,upsert)。
  - tool_takeover 强制 enabled=0。
- [ ] `mod.rs`:`export(mode, password?)`、`import(package, password?, conflict_mode)` 编排;full_backup 导入前本地 DB 备份。
- [ ] `services/mod.rs` 注册 `pub mod portability;`。

### 3. HTTP API `http/api/settings.rs` 扩展

- [ ] `POST /api/settings/export` body `{mode, password?}` → `{package, warnings?}`;主密钥不可用 503,缺密码 400。
- [ ] `POST /api/settings/import` body `{package, password?, conflict_mode?}` → `{imported:{...}, warnings?}`;解密失败/版本不符 400。
- [ ] 复用现有 settings `routes()`,追加两条 POST。

### 4. 前端

- [ ] `lib/api.ts`:`portabilityApi`(exportConfig/importConfig)+ 类型。
- [ ] `SettingsPage.tsx`:新增「配置导入导出」卡片——完整备份导出、脱敏导出(密码+弱密码提示)、导入(文件选择+密码+冲突提示)、风险提示文案。
- [ ] 导出下载用 Blob + a[download];导入用 FileReader 读文本。

### 5. 质量门

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo fmt && cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cd .. && npm run build
```

### 6. 运行验证(隔离 HOME)

- [ ] 完整备份导出 → 含加密 BLOB;同主密钥环境导入 → 凭据恢复;接管全部关闭。
- [ ] 脱敏导出(设密码)→ 无凭据列;换环境导入(输密码)→ 配置 merge,凭据缺失需重录。
- [ ] 密码错误导入 → 可读错误,不崩溃。
- [ ] version 不符 → 拒绝。
- [ ] 导入事务回滚:构造损坏包验证不留半成品。
- [ ] request_logs / 测试数据未被导出。
- [ ] 弱密码 → warning。

## 风险与回滚点

- **风险:导入破坏现有配置**。缓解:full_backup 导入前自动 DB 文件备份 + 全程事务。
- **风险:明文凭据暴露**。缓解:full_backup 装入已加密 BLOB,不解密重装;脱敏包绝不含凭据。
- **回滚**:全是新增模块 + settings API 追加 + SettingsPage 扩展;回滚即 git checkout。

## 实现方式

worktree sub-agent 实现(参考前 4 个子任务流程);完成后主 session 合并 + 质量门 + 运行验证。
