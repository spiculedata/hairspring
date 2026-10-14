use anthill_engine::EngineError;
use anthill_store::StoreError;
use std::fmt;

#[derive(Debug)]
pub enum ServiceError {
    /// The caller sent something invalid; safe to show to them.
    InvalidRequest(String),
    NotFound(String),
    Engine(EngineError),
    Store(StoreError),
    Internal(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::InvalidRequest(m) => write!(f, "invalid request: {m}"),
            ServiceError::NotFound(w) => write!(f, "not found: {w}"),
            ServiceError::Engine(e) => write!(f, "{e}"),
            ServiceError::Store(e) => write!(f, "{e}"),
            ServiceError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<EngineError> for ServiceError {
    fn from(e: EngineError) -> Self {
        ServiceError::Engine(e)
    }
}

impl From<StoreError> for ServiceError {
    fn from(e: StoreError) -> Self {
        ServiceError::Store(e)
    }
}
