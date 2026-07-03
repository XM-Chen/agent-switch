pub mod codex_oauth;
pub mod crypto;
pub mod keychain;
pub mod model_alias;
pub mod model_sync;
/// 配置导入导出服务（本机加密完整备份 / 可迁移脱敏配置导出）。
pub mod portability;
/// Provider 领域层：ccs 式统一切换单元（与 accounts+endpoints 并存）。
/// 供 HTTP API 与工具接管消费；proxy 桥接接入前部分类型暂未被引用。
#[allow(dead_code)]
pub mod provider;
pub mod tool_takeover;
/// 本模块供 proxy 层消费。proxy 层尚未实现时标记该模块为 unused 是错误的。
/// 将在 proxy 模块接入后移除该属性。
#[allow(dead_code)]
pub mod translator;
