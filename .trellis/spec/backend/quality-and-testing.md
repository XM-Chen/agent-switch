# Rust 质量与测试

## 必跑门禁

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo check --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri build --no-bundle
```

Windows-only 产品每个裁剪批次都在 Windows 执行；Linux CI 通过不能替代 Windows cfg 测试。

## 代码质量

- command 薄、Service 编排、DAO 持久化、adapter 文件/协议；不跨层偷写。
- 生产路径不用 `unwrap/expect/panic`；锁中毒、IO、JSON、网络均映射 `AppError`。
- Clippy 以 `-D warnings` 为完成门，不用全局 allow 掩盖问题；平台专属函数用正确 `#[cfg]`。
- 路径从集中函数派生；Windows 不随意使用 `HOME`。
- JSON 未知字段保留；需要 typed view 时以原 Value 为底更新，不做破坏性 round-trip。
- 日志不含 token、私钥、完整 settings 或同步 SQL。

## 测试层次

1. 模块内 `#[cfg(test)]`：纯函数、转换、DAO memory DB；
2. `src-tauri/tests/`：command/service/迁移/配置集成；
3. 临时 HOME：live 文件、身份隔离、首次启动；
4. Axum router：HTTP/status/auth/stream/failover；
5. Windows no-bundle build：链接与 cfg 完整性。

涉及环境变量/全局路径覆盖时使用 tempdir + serial lock，测试结束恢复环境。测试 token 必须是假值。

## 基线已知问题

ccs v3.16.5 Windows 原样门存在：

- Clippy 13 errors（Windows 下 Unix shell 辅助函数 dead_code、needless_return、unused import）；
- 8 个 Rust 测试失败（6 Codex Windows upgrade、1 codex_history、1 Claude Desktop sync）；
- `cargo check` 和 `tauri build --no-bundle` 通过。

完整证据见 `../../tasks/07-10-ccs-baseline-bootstrap/research/r3-validation-results.md`。

裁剪后不能永久豁免：

- 独立 Codex/非 Windows 路径被删除时对应失败必须归零；
- Claude Desktop sync 是否保留需按 Claude 主链引用闭包决定；若删除 Desktop 客户端，确保不误删通用 gateway/Provider 能力；
- 最终 clippy/test 必须全绿。

## 构建缓存

同一 target 目录交替 `cargo check` 与 `cargo test` 曾出现 `.rmeta/.rlib` E0786 冲突。发生时先判定缓存而非代码，完整 `cargo clean` 后重跑；长链路可为 check/test 分配不同 `CARGO_TARGET_DIR`。不要把缓存错误记录成产品回归。

## 完成定义

Rust 改动需：格式、clippy、全量 test、check、相关集成测试和 Windows no-bundle build 通过；若基线问题仍属后续批次，报告必须精确到测试名/行/归属任务，不得写笼统“环境问题”。
