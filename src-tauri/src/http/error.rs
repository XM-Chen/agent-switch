use axum::http::StatusCode;
use axum::response::Json;
use serde::Serialize;

/// Unified error response for placeholder / unimplemented scopes.
#[derive(Serialize)]
pub struct PlaceholderError {
    pub error: PlaceholderErrorDetail,
}

#[derive(Serialize)]
pub struct PlaceholderErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub message: String,
    pub scope: String,
}

/// Generate a `501 Not Implemented` response for a given scope.
pub fn placeholder_error(scope: &str) -> (StatusCode, Json<PlaceholderError>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(PlaceholderError {
            error: PlaceholderErrorDetail {
                error_type: "not_implemented".to_string(),
                code: "scope_not_ready".to_string(),
                message: "该入口已预留，但当前子任务尚未实现具体功能。".to_string(),
                scope: scope.to_string(),
            },
        }),
    )
}
