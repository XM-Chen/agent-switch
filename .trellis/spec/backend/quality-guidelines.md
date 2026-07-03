# Rust 后端质量规范

## 必跑命令

```bash
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

## 测试要求

- 修复协议转换时必须新增 translator 单测覆盖请求映射和流式事件顺序。
- 修复 proxy/failover 时必须覆盖 retry/cooldown/stream guard/SSE 终止语义。
- 修复 DAO/portability 时必须覆盖事务语义、round-trip、冲突/孤儿数据处理。
- 修复 OAuth 时必须覆盖 callback 早退、JWT 解析、账号 ID 稳定性和状态码分类。

## 日志与敏感信息

测试和日志不得输出 API key、OAuth token、Authorization header、prompt/messages 或完整请求/响应正文。

## 格式

功能子任务可不单独运行 `cargo fmt`，但最终收敛任务必须运行 `cargo fmt` 与 `cargo fmt --check`。
