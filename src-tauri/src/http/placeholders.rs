use axum::extract::Request;

/// Fallback handler for `/claude-code/`, `/codex/` etc — always 501.
/// claude-code and codex have been replaced by RouteProxy in router.rs.
pub async fn not_implemented(request: Request) -> impl axum::response::IntoResponse {
    let path = request.uri().path();
    let scope = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("unknown");

    super::error::placeholder_error(scope)
}
