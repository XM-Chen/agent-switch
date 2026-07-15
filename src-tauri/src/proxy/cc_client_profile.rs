//! Claude Code 客户端 header profile（L1 兼容规整）单一事实源。
//!
//! # 这是什么
//!
//! 一份随 Agent Switch 版本内置、经交叉验证的 Claude CLI/SDK 出站 header 集合。
//! 当「Claude 客户端指纹规整」开关开启且当前候选是 native Anthropic 时，forwarder
//! 用这份 profile 强制统一 `user-agent` / `x-app` / `x-stainless-*`，让来自第三方
//! 客户端（OpenCode、Cline、Pi 等）的请求不再泄露它们自己的客户端指纹。
//!
//! # 重要边界
//!
//! - 这些值属于 **非公开的 Claude Code / Stainless SDK 客户端实现特征**，
//!   **不是** Anthropic Messages API 的稳定契约。上游可能随时变化对它们的处理。
//! - 本模块只负责 header 规整；**不含** `authorization` / `x-api-key` /
//!   `anthropic-beta` / `anthropic-version` / `anthropic-dangerous-direct-browser-access`，
//!   这些仍由 forwarder 现有逻辑负责（dangerous header 由守卫按鉴权策略单独处理）。
//! - 运行时不联网更新、不依赖本机是否安装 Claude Code。profile 更新走正常发版。
//!
//! # 来源与验证（profile 冻结记录）
//!
//! - profile_id：`claude-cli-2.1.210-20260715`
//! - 冻结日期：2026-07-15
//! - Claude CLI 版本 `2.1.210`：**已验证**。本机 `claude --version` 输出
//!   `2.1.210 (Claude Code)`；`@anthropic-ai/claude-code@2.1.210` 二进制内嵌
//!   字符串同时含 `claude-cli/`、`2.1.210`、` (external, `、`cli`。
//! - user-agent `claude-cli/2.1.210 (external, cli)`：由上述二进制相邻字符串
//!   片段拼接推定，与 Claude Code 已知 UA 格式一致。**非抓包**，是对内嵌字符串
//!   的交叉核对。
//! - x-app `cli`：二进制内嵌 `cli`（与 `claude-cli/` 相邻），与参考项目一致。
//! - x-stainless-runtime `node` / runtime-version `v26.3.0`：二进制内嵌
//!   `node/v26.3.0 win32 x64`，为 2.1.210 打包内嵌的 Node 运行时版本。
//! - x-stainless-os `Windows` / x-stainless-arch `x64`：Stainless JS SDK 对
//!   `process.platform=win32` / `process.arch=x64` 的规范化取值。本项目为
//!   Windows-only 发行，故冻结为该组合。
//! - x-stainless-lang `js`：`@anthropic-ai/claude-code` 基于 `@anthropic-ai/sdk`
//!   (JS/TS SDK)，Stainless lang 取 `js`。
//! - x-stainless-package-version `0.111.0`：`@anthropic-ai/sdk` 当前发布版本
//!   （npm 查询）。**限制**：claude-code 2.1.210 为编译产物、依赖树为空，无法从
//!   包元数据直接确认其内嵌 SDK 版本恰为 0.111.0；此处取当前可验证的 SDK 版本，
//!   若上游对 package-version 敏感，用户可用 provider 级 `customUserAgent` /
//!   `localProxyRequestOverrides` 临时纠偏。
//! - x-stainless-retry-count `0`：首次尝试的取值（Stainless 每次重试自增）。
//!   这里固定为初始值，不随本代理内部候选切换而变化。
//! - x-stainless-timeout `60`：**未直接抓取到确切值**，取一个最小且自洽的
//!   保守默认（秒）。列入是为满足 profile 字段完整性；对上游一般不敏感。

/// Claude CLI/SDK 出站 header profile（仅 header，不含身份/计费/认证字段）。
pub struct CcClientProfile {
    /// profile 标识（含 CLI 版本与冻结日期）；用于 debug 日志可观测性（R6）。
    pub profile_id: &'static str,
    /// Claude CLI 版本（用于与 UA 中版本号一致性校验）。
    /// 仅在一致性单测中读取，运行时不直接消费，故标记 allow。
    #[allow(dead_code)]
    pub cli_version: &'static str,
    /// `user-agent` 值
    pub user_agent: &'static str,
    /// `x-app` 值
    pub x_app: &'static str,
    /// `x-stainless-*` header 集合，key 全小写。
    pub stainless: &'static [(&'static str, &'static str)],
}

/// 内置默认 profile（当前唯一一份）。
pub const CC_PROFILE_2_1_210: CcClientProfile = CcClientProfile {
    profile_id: "claude-cli-2.1.210-20260715",
    cli_version: "2.1.210",
    user_agent: "claude-cli/2.1.210 (external, cli)",
    x_app: "cli",
    stainless: &[
        ("x-stainless-lang", "js"),
        ("x-stainless-package-version", "0.111.0"),
        ("x-stainless-os", "Windows"),
        ("x-stainless-arch", "x64"),
        ("x-stainless-runtime", "node"),
        ("x-stainless-runtime-version", "v26.3.0"),
        ("x-stainless-retry-count", "0"),
        ("x-stainless-timeout", "60"),
    ],
};

/// 返回当前生效的内置 profile。
///
/// 目前只有一份内置 profile；保留函数入口以便将来扩展为可选 profile。
#[inline]
pub fn active_profile() -> &'static CcClientProfile {
    &CC_PROFILE_2_1_210
}

/// 该 header 是否由内置 profile 托管（`x-app` 或任意 `x-stainless-*`）。
///
/// forwarder 在 `apply_client_profile` 为真时用它把入站同名头全部丢弃，
/// 由 `apply_client_profile_headers` 统一写入规范值，避免重复/残留。
/// `user-agent` 单独处理（要与 `customUserAgent` 交互），不在此判据内。
/// `anthropic-dangerous-direct-browser-access` 也单独按鉴权策略处理。
pub fn is_client_profile_managed_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("x-app") || name.to_ascii_lowercase().starts_with("x-stainless-")
}

/// 把内置 profile 托管的 header 统一写入 `headers`。这是 forwarder 应用
/// client profile 的唯一写入点，stream / 非 stream、reqwest / hyper 两条发送
/// 路径共用同一个 `HeaderMap`，因此构造上保证两路径最终 header 集合等价。
///
/// 写入规则：
/// - `user-agent`：`custom_user_agent`（provider 级显式配置）优先（D7）；
///   否则用 profile UA。调用方须保证入站 UA 已被丢弃（避免重复）。
/// - `x-app` / `x-stainless-*`：强制用 profile 值覆盖（D7）。
/// - `anthropic-dangerous-direct-browser-access`：仅当 `inject_dangerous=true`
///   （即候选实际用 `AuthStrategy::Anthropic` 静态 x-api-key）时写 `true`；否则
///   不写（入站残留由调用方在循环内丢弃）（D8）。
///
/// 使用 `insert` 语义：同名头只保留一个规范值，不会与残留入站头并存。
pub fn append_client_profile_headers(
    headers: &mut http::HeaderMap,
    profile: &CcClientProfile,
    custom_user_agent: Option<&http::HeaderValue>,
    inject_dangerous: bool,
) {
    // user-agent：customUserAgent 优先，否则 profile UA
    if let Some(ua) = custom_user_agent {
        headers.insert(http::header::USER_AGENT, ua.clone());
    } else if let Ok(hv) = http::HeaderValue::from_str(profile.user_agent) {
        headers.insert(http::header::USER_AGENT, hv);
    }

    // x-app
    if let Ok(hv) = http::HeaderValue::from_str(profile.x_app) {
        headers.insert(http::HeaderName::from_static("x-app"), hv);
    }

    // x-stainless-*
    for (sk, sv) in profile.stainless {
        if let (Ok(name), Ok(hv)) = (
            http::HeaderName::from_bytes(sk.as_bytes()),
            http::HeaderValue::from_str(sv),
        ) {
            headers.insert(name, hv);
        }
    }

    // anthropic-dangerous-direct-browser-access（D8）
    if inject_dangerous {
        headers.insert(
            http::HeaderName::from_static("anthropic-dangerous-direct-browser-access"),
            http::HeaderValue::from_static("true"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_client_profile_fields_non_empty() {
        let p = active_profile();
        assert!(!p.profile_id.is_empty());
        assert!(!p.cli_version.is_empty());
        assert!(!p.user_agent.is_empty());
        assert!(!p.x_app.is_empty());
        assert!(!p.stainless.is_empty());
        for (k, v) in p.stainless {
            assert!(!k.is_empty(), "stainless header 名不能为空");
            assert!(!v.is_empty(), "stainless header 值不能为空: {k}");
            assert_eq!(
                *k,
                k.to_ascii_lowercase(),
                "stainless header 名必须全小写: {k}"
            );
        }
    }

    #[test]
    fn cc_client_profile_ua_contains_cli_version() {
        // 教训（见 reference-projects 调研）：UA 版本号必须与 cli_version 一致，
        // 否则上游一眼看穿是拼装指纹。
        let p = active_profile();
        assert!(
            p.user_agent.contains(p.cli_version),
            "user-agent ({}) 必须包含 cli_version ({})",
            p.user_agent,
            p.cli_version
        );
        assert!(
            p.user_agent.starts_with("claude-cli/"),
            "user-agent 必须以 claude-cli/ 开头"
        );
    }

    #[test]
    fn cc_client_profile_x_app_is_cli() {
        assert_eq!(active_profile().x_app, "cli");
    }

    #[test]
    fn cc_client_profile_covers_required_stainless_headers() {
        let p = active_profile();
        let required = [
            "x-stainless-lang",
            "x-stainless-package-version",
            "x-stainless-os",
            "x-stainless-arch",
            "x-stainless-runtime",
            "x-stainless-runtime-version",
            "x-stainless-retry-count",
            "x-stainless-timeout",
        ];
        for name in required {
            assert!(
                p.stainless.iter().any(|(k, _)| *k == name),
                "profile 缺少必需的 stainless header: {name}"
            );
        }
    }

    #[test]
    fn managed_header_predicate() {
        assert!(is_client_profile_managed_header("x-app"));
        assert!(is_client_profile_managed_header("X-App"));
        assert!(is_client_profile_managed_header("x-stainless-lang"));
        assert!(is_client_profile_managed_header("X-Stainless-Timeout"));
        // UA 与 dangerous 单独处理，不算 managed
        assert!(!is_client_profile_managed_header("user-agent"));
        assert!(!is_client_profile_managed_header(
            "anthropic-dangerous-direct-browser-access"
        ));
        assert!(!is_client_profile_managed_header("anthropic-beta"));
    }

    #[test]
    fn append_headers_uses_profile_ua_and_stainless() {
        let p = active_profile();
        let mut headers = http::HeaderMap::new();
        append_client_profile_headers(&mut headers, p, None, false);

        assert_eq!(headers.get(http::header::USER_AGENT).unwrap(), p.user_agent);
        assert_eq!(headers.get("x-app").unwrap(), p.x_app);
        for (k, v) in p.stainless {
            assert_eq!(headers.get(*k).unwrap(), *v, "stainless header {k} 应写入");
        }
        // 未开 dangerous：不应出现该头
        assert!(headers
            .get("anthropic-dangerous-direct-browser-access")
            .is_none());
    }

    #[test]
    fn append_headers_custom_ua_wins_over_profile() {
        let p = active_profile();
        let mut headers = http::HeaderMap::new();
        let custom = http::HeaderValue::from_static("my-custom-agent/1.0");
        append_client_profile_headers(&mut headers, p, Some(&custom), false);

        // customUserAgent 显式配置优先于 profile UA（D7）
        assert_eq!(
            headers.get(http::header::USER_AGENT).unwrap(),
            "my-custom-agent/1.0"
        );
        // x-app / x-stainless 仍是 profile 值
        assert_eq!(headers.get("x-app").unwrap(), p.x_app);
    }

    #[test]
    fn append_headers_dangerous_only_when_injected() {
        let p = active_profile();
        let mut headers = http::HeaderMap::new();
        append_client_profile_headers(&mut headers, p, None, true);
        assert_eq!(
            headers
                .get("anthropic-dangerous-direct-browser-access")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn append_headers_overwrites_existing_managed_values() {
        let p = active_profile();
        let mut headers = http::HeaderMap::new();
        // 模拟入站残留（正常路径会在循环内丢弃，这里直接测 insert 覆盖语义）
        headers.insert(
            http::header::USER_AGENT,
            http::HeaderValue::from_static("opencode/9.9.9"),
        );
        headers.insert(
            http::HeaderName::from_static("x-app"),
            http::HeaderValue::from_static("opencode"),
        );
        append_client_profile_headers(&mut headers, p, None, false);

        // insert 语义：只保留一个规范值
        assert_eq!(headers.get_all(http::header::USER_AGENT).iter().count(), 1);
        assert_eq!(headers.get(http::header::USER_AGENT).unwrap(), p.user_agent);
        assert_eq!(headers.get_all("x-app").iter().count(), 1);
        assert_eq!(headers.get("x-app").unwrap(), p.x_app);
    }
}
