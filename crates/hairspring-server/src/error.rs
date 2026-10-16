use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_engine::EngineError;
use hairspring_service::ServiceError;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    www_authenticate: bool,
    retry_after_secs: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            message: message.into(),
            www_authenticate: false,
            retry_after_secs: None,
        }
    }
    pub fn bad_request(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn not_found() -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not found")
    }
    pub fn unauthorized() -> ApiError {
        ApiError {
            www_authenticate: true,
            ..ApiError::new(StatusCode::UNAUTHORIZED, "a valid bearer token is required")
        }
    }
    /// A 429 that tells the caller when to try again.
    pub fn too_many_requests(message: impl Into<String>, retry_after_secs: u64) -> ApiError {
        ApiError {
            retry_after_secs: Some(retry_after_secs),
            ..ApiError::new(StatusCode::TOO_MANY_REQUESTS, message)
        }
    }
    pub fn forbidden(needed: &str) -> ApiError {
        ApiError::new(
            StatusCode::FORBIDDEN,
            format!("this endpoint requires the {needed} role"),
        )
    }
    /// Logs the detail server-side and returns a generic message: internals never reach the caller.
    pub fn internal(detail: impl std::fmt::Display) -> ApiError {
        eprintln!("hairspring-server: internal error: {detail}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> ApiError {
        match e {
            ServiceError::InvalidRequest(m) => ApiError::bad_request(m),
            ServiceError::NotFound(_) => ApiError::not_found(),
            ServiceError::Engine(EngineError::Backend(m)) => ApiError::internal(m),
            ServiceError::Engine(e) => ApiError::bad_request(e.to_string()),
            ServiceError::Store(e) => ApiError::internal(e),
            ServiceError::Internal(m) => ApiError::internal(m),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (self.status, Json(json!({ "error": self.message }))).into_response();
        if self.www_authenticate {
            res.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                header::HeaderValue::from_static("Bearer"),
            );
        }
        if let Some(secs) = self.retry_after_secs {
            res.headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from(secs));
        }
        res
    }
}
