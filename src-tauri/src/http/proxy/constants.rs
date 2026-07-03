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

// ── OAuth ────────────────────────────────────────────────
/// OAuth access_token 过期前提前刷新时间（秒）。
pub const OAUTH_REFRESH_LEAD_TIME_SECS: i64 = 60;
/// OAuth 刷新请求总超时（秒）。
///
/// 刷新在代理主循环预检阶段执行；auth.openai.com 半挂时必须尽快返回，
/// 否则会永久阻塞故障转移主循环。总超时控制在 30s 内。
pub const OAUTH_REFRESH_TIMEOUT_SECS: u64 = 30;
/// OAuth 刷新连接超时（秒）。
pub const OAUTH_REFRESH_CONNECT_TIMEOUT_SECS: u64 = 10;

// ── 冷却 ────────────────────────────────────────────────
/// 429 Retry-After 额外缓冲（秒），在上游值基础上加。
pub const COOLDOWN_429_RETRY_AFTER_BUFFER_SECS: i64 = 1;
/// 指数退避最大冷却时长（秒）。
pub const COOLDOWN_MAX_EXPONENTIAL_SECS: u64 = 300;
/// 5xx 通用冷却基准（秒）。
pub const COOLDOWN_5XX_BASE_SECS: i64 = 30;
/// 5xx 通用冷却上限（秒）。
pub const COOLDOWN_5XX_CAP_SECS: i64 = 120;
/// 5xx 每次切换递增的冷却（秒）。
pub const COOLDOWN_5XX_STEP_SECS: i64 = 15;
/// 未分类瞬时错误默认冷却（秒）。仅用于 `calculate_cooldown_seconds` 默认分支。
pub const COOLDOWN_DEFAULT_SECS: i64 = 30;
/// AuthError（OAuth 刷新失败）冷却时长（秒）。
///
/// PRD 第 92 行规定 auth 类冷却 5 分钟；30s 会令 selector 反复重选已失效
/// 凭据端点并触发刷新失败，放大上游错误率。
pub const COOLDOWN_AUTH_ERROR_SECS: i64 = 300;
/// 网络错误冷却时长（秒）。
pub const COOLDOWN_NETWORK_ERROR_SECS: i64 = 30;
/// 超时错误冷却时长（秒）。
pub const COOLDOWN_TIMEOUT_SECS: i64 = 60;
/// 429 缺失 Retry-After 时的默认冷却（秒）。
pub const COOLDOWN_429_DEFAULT_RETRY_AFTER_SECS: i64 = 5;
/// 429 最小冷却（秒）。
pub const COOLDOWN_429_MIN_SECS: i64 = 1;
/// 429 冷却上限（秒），与指数退避上限对齐防止失控。
pub const COOLDOWN_429_CAP_SECS: i64 = 300;
/// 503/529 指数退避最大幂次（防止 2^N 溢出）。
pub const COOLDOWN_EXPONENTIAL_MAX_POWER: u32 = 8;

// ── 同端点重试 ──────────────────────────────────────────
/// 同端点重试间隔（毫秒）。
///
/// PRD R3.3 / route policy 指定 retryDelay=500ms；可重试错误返回后，
/// 同一端点的下一次重试应至少延迟此值再发，避免 hammer 上游。
pub const SAME_ACCOUNT_RETRY_DELAY_MS: u64 = 500;
