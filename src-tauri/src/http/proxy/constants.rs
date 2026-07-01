//! 路由与故障转移核心常量。
//!
//! 参考设计文档 §4「常量与路径契约」。
//! 协议类常量同时用作 `protocol_type` 校验取值。
//!
//! 部分默认参数常量为路由管理层预留，尚未在主循环接线，
//! 保留以对齐设计契约。
#![allow(dead_code)]

// ── 协议类 ──────────────────────────────────────────────
/// Anthropic Messages API 协议。
pub const PROTOCOL_ANTHROPIC: &str = "anthropic";
/// OpenAI Chat Completions API 协议。
pub const PROTOCOL_OPENAI_CHAT: &str = "openai-chat";
/// OpenAI Responses API 协议。
pub const PROTOCOL_OPENAI_RESPONSES: &str = "openai-responses";
/// OpenAI-compatible v1 多协议路由（仅用于 route_settings，不用于 endpoint 管理 UI 校验）。
pub const PROTOCOL_OPENAI_COMPATIBLE: &str = "openai-compatible";

// ── 选择策略 ────────────────────────────────────────────
/// 按优先级固定顺序选择，失败后递进下一候选。
pub const FILL_FIRST: &str = "fill-first";
/// 在相同优先级内轮询，带粘性会话。
pub const ROUND_ROBIN: &str = "round-robin";

// ── 故障转移默认参数 ────────────────────────────────────
/// 候选链最大切换次数。
pub const DEFAULT_MAX_SWITCHES: u32 = 10;
/// 同端点最大重试次数。
pub const DEFAULT_SAME_ACCOUNT_RETRIES: u32 = 3;
/// 同端点重试间隔（毫秒）。
pub const SAME_ACCOUNT_RETRY_DELAY_MS: u64 = 500;

// ── OAuth ────────────────────────────────────────────────
/// OAuth access_token 过期前提前刷新时间（秒）。
pub const OAUTH_REFRESH_LEAD_TIME_SECS: i64 = 60;

// ── 冷却 ────────────────────────────────────────────────
/// 429 Retry-After 额外缓冲（秒），在上游值基础上加。
pub const COOLDOWN_429_RETRY_AFTER_BUFFER_SECS: i64 = 1;
/// 指数退避最大冷却时长（秒）。
pub const COOLDOWN_MAX_EXPONENTIAL_SECS: u64 = 300;
