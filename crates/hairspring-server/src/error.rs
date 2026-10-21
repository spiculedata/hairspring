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
    /// A stable identifier a client can branch on, when the error has one.
    code: Option<&'static str>,
}

/// The `code` of a replay refused because the host's dynamic loader differs from the recording.
pub const ENVIRONMENT_MISMATCH_CODE: &str = "environment_mismatch";

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            message: message.into(),
            www_authenticate: false,
            retry_after_secs: None,
            code: None,
        }
    }
    /// The JSON body: the short text under `error`, plus `code` when there is one.
    pub fn body(&self) -> serde_json::Value {
        match self.code {
            Some(code) => json!({ "error": self.message, "code": code }),
            None => json!({ "error": self.message }),
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
            // The caller can fix this (retry with the override), so say what it is, briefly.
            ServiceError::Engine(e @ EngineError::EnvironmentMismatch(_)) => ApiError {
                code: Some(ENVIRONMENT_MISMATCH_CODE),
                ..ApiError::new(
                    StatusCode::CONFLICT,
                    hairspring_service::sanitize::public_text(&e.to_string()),
                )
            },
            ServiceError::Engine(e) => ApiError::bad_request(e.to_string()),
            ServiceError::Store(e) => ApiError::internal(e),
            ServiceError::Internal(m) => ApiError::internal(m),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (self.status, Json(self.body())).into_response();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_mismatch_is_a_409_with_a_short_single_line_message_not_an_internal_error() {
        let hostile = format!("environment differs: {}\n\u{202E}tail", "x".repeat(5000));
        let e = ApiError::from(ServiceError::Engine(EngineError::EnvironmentMismatch(
            hostile,
        )));
        assert_eq!(e.status, StatusCode::CONFLICT);
        assert!(
            e.message.starts_with("environment mismatch"),
            "{}",
            e.message
        );
        assert!(e.message.chars().count() <= 500, "{}", e.message.len());
        assert!(!e.message.contains('\n') && !e.message.contains('\u{202E}'));
        assert_ne!(e.message, "internal error");
    }

    #[test]
    fn the_409_body_carries_a_machine_readable_code_besides_the_text() {
        let e = ApiError::from(ServiceError::Engine(EngineError::EnvironmentMismatch(
            "environment differs: host.loader.libc.sha256".into(),
        )));
        let body = e.body();
        assert_eq!(body["code"], "environment_mismatch");
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("environment mismatch"));
        assert!(ApiError::bad_request("x").body().get("code").is_none());
    }

    #[test]
    fn other_backend_errors_stay_internal() {
        let e = ApiError::from(ServiceError::Engine(EngineError::Backend("boom".into())));
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
