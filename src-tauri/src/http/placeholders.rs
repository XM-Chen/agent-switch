use axum::{extract::Request, response::IntoResponse};

/// Fallback handler for any unimplemented scope.
/// Returns a 501 JSON error without referencing the request body.
pub async fn not_implemented(_request: Request) -> impl IntoResponse {
    // Extract the first path segment to determine the scope
    let path = _request.uri().path();
    let scope = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("unknown");

    super::error::placeholder_error(scope)
}
