use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("rate limit exceeded")]
    RateLimited,
    /// Too many attempts at a protected action, with a user-facing message.
    #[error("{0}")]
    TooManyAttempts(String),
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    /// A refused action with a stable error code and a user-facing message.
    #[error("{1}")]
    Refused(&'static str, String),
    /// A gateway refusal with its own status and a stable code that says
    /// which key rule stopped the request.
    #[error("{2}")]
    Coded(StatusCode, &'static str, String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for AppError {
    fn from(value: sqlx::Error) -> Self {
        tracing_error(value.to_string())
    }
}

fn tracing_error(message: String) -> AppError {
    AppError::Internal(message)
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            AppError::RateLimited | AppError::TooManyAttempts(_) => {
                (StatusCode::TOO_MANY_REQUESTS, "rate_limited")
            }
            AppError::ServiceUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable")
            }
            AppError::Refused(code, _) => (StatusCode::FORBIDDEN, code),
            AppError::Coded(status, code, _) => (status, code),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let message = match &self {
            AppError::Internal(_) => "internal server error".to_owned(),
            _ => self.to_string(),
        };
        (
            status,
            Json(json!({"error": {"code": code, "message": message}})),
        )
            .into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
