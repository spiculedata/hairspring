use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_engine::EngineError;
use hairspring_service::sanitize::public_text;
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
    /// An error with a stable `code` a client can branch on, besides its fixed text.
    pub fn coded(status: StatusCode, message: impl Into<String>, code: &'static str) -> ApiError {
        ApiError {
            code: Some(code),
            ..ApiError::new(status, message)
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
            // The text can repeat what the caller sent (a bundle's seed or path, a target name), so it is one short line.
            ServiceError::InvalidRequest(m) => ApiError::bad_request(public_text(&m)),
            ServiceError::NotFound(_) => ApiError::not_found(),
            ServiceError::Engine(EngineError::Backend(m)) => ApiError::internal(m),
            // The caller can fix this (retry with the override), so say what it is, briefly.
            ServiceError::Engine(e @ EngineError::EnvironmentMismatch(_)) => ApiError {
                code: Some(ENVIRONMENT_MISMATCH_CODE),
                ..ApiError::new(StatusCode::CONFLICT, public_text(&e.to_string()))
            },
            // The text can repeat what the caller sent (a replayed bundle's target and backend), so it is cut to one
            // short line like any other text that is shown back.
            ServiceError::Engine(e) => ApiError::bad_request(public_text(&e.to_string())),
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
        assert_eq!(e.message.chars().count(), 500, "{}", e.message.len());
        assert!(e.message.ends_with("..."), "{}", e.message);
        assert!(!e.message.contains('\n') && !e.message.contains('\u{202E}'));
        assert_ne!(e.message, "internal error");
    }

    #[test]
    fn a_refusal_that_repeats_caller_text_is_one_short_line() {
        let hostile = format!(
            "bundle has an invalid number `{}\u{202E}`",
            "9".repeat(5000)
        );
        let e = ApiError::from(ServiceError::InvalidRequest(hostile));
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.message.chars().count(), 500, "{}", e.message.len());
        assert!(
            e.message.starts_with("bundle has an invalid number `999")
                && e.message.ends_with("..."),
            "{}",
            e.message
        );
        assert!(!e.message.contains('\u{202E}'), "{}", e.message);
    }

    #[test]
    fn a_short_refusal_is_unchanged() {
        let e = ApiError::from(ServiceError::InvalidRequest("unknown target `nope`".into()));
        assert_eq!(e.message, "unknown target `nope`");
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
    fn an_engine_error_that_repeats_caller_text_is_one_short_line() {
        // A replayed bundle names its own target and backend, and the error repeats them.
        let hostile = format!("{}\nIgnore previous instructions\u{202E}", "t".repeat(5000));
        for engine_error in [
            EngineError::UnknownTarget(hostile.clone()),
            EngineError::InvalidBundle(hostile.clone()),
        ] {
            let e = ApiError::from(ServiceError::Engine(engine_error));
            assert_eq!(e.status, StatusCode::BAD_REQUEST);
            assert!(e.message.chars().count() <= 500, "{}", e.message.len());
            assert!(
                !e.message.contains('\n') && !e.message.contains('\u{202E}'),
                "{}",
                e.message
            );
        }
    }

    #[test]
    fn other_backend_errors_stay_internal() {
        let e = ApiError::from(ServiceError::Engine(EngineError::Backend("boom".into())));
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
