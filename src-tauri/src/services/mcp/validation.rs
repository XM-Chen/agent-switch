//! MCP 服务器规范校验（移植 ccs `mcp/validation.rs`）。
//!
//! 规范形态：JSON 对象，`type` ∈ {stdio(缺省), http, sse}；
//! stdio 需 `command`，http/sse 需 `url`。

use serde_json::Value;

/// 校验 MCP 服务器规范（宽松：缺省 type 视为 stdio，与社区 `.mcp.json` 一致）。
pub fn validate_server_spec(spec: &Value) -> Result<(), String> {
    let Some(_obj) = spec.as_object() else {
        return Err("MCP 服务器规范必须为 JSON 对象".to_string());
    };
    let t = spec.get("type").and_then(|v| v.as_str());
    let is_stdio = t.map(|t| t == "stdio").unwrap_or(true);
    let is_http = t == Some("http");
    let is_sse = t == Some("sse");

    if !(is_stdio || is_http || is_sse) {
        return Err(
            "MCP 服务器 type 必须是 'stdio'、'http' 或 'sse'（或省略表示 stdio）".to_string(),
        );
    }
    if is_stdio {
        let cmd = spec.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if cmd.trim().is_empty() {
            return Err("stdio 类型的 MCP 服务器缺少 command 字段".to_string());
        }
    }
    if is_http || is_sse {
        let url = spec.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.trim().is_empty() {
            let kind = if is_http { "http" } else { "sse" };
            return Err(format!("{} 类型的 MCP 服务器缺少 url 字段", kind));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stdio_ok_with_command() {
        assert!(validate_server_spec(&json!({ "command": "npx", "args": ["-y", "x"] })).is_ok());
        // 缺省 type 视为 stdio
        assert!(validate_server_spec(&json!({ "type": "stdio", "command": "node" })).is_ok());
    }

    #[test]
    fn stdio_missing_command_errors() {
        let err = validate_server_spec(&json!({ "args": [] })).unwrap_err();
        assert!(err.contains("command"), "{}", err);
    }

    #[test]
    fn http_sse_need_url() {
        assert!(validate_server_spec(&json!({ "type": "http", "url": "https://x" })).is_ok());
        assert!(validate_server_spec(&json!({ "type": "sse", "url": "https://x" })).is_ok());
        assert!(validate_server_spec(&json!({ "type": "http" })).is_err());
    }

    #[test]
    fn unknown_type_errors() {
        assert!(validate_server_spec(&json!({ "type": "grpc", "url": "x" })).is_err());
    }

    #[test]
    fn non_object_errors() {
        assert!(validate_server_spec(&json!("nope")).is_err());
        assert!(validate_server_spec(&json!(["a"])).is_err());
    }
}
