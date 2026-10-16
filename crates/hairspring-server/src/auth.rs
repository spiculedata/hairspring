use crate::error::ApiError;
use crate::AppState;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use hairspring_service::Role;
use sha2::{Digest, Sha256};

const MIN_TOKEN_LEN: usize = 16;

/// Bearer-token authentication. Tokens are kept only as SHA-256 digests and compared
/// in constant time across every configured token.
#[derive(Clone, Default)]
pub struct Auth {
    tokens: Vec<([u8; 32], Role)>,
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Auth {
    /// No authentication (loopback development only; `start` enforces that).
    pub fn open() -> Auth {
        Auth::default()
    }

    pub fn is_open(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Parses `token:role,token:role`. Tokens must be at least 16 characters.
    pub fn from_spec(spec: &str) -> Result<Auth, String> {
        let mut tokens: Vec<([u8; 32], Role)> = Vec::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (token, role) = item
                .rsplit_once(':')
                .ok_or("each token needs a role: token:role")?;
            if token.len() < MIN_TOKEN_LEN
                || token.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(format!(
                    "tokens must be at least {MIN_TOKEN_LEN} characters with no whitespace"
                ));
            }
            let role = Role::parse(role)
                .ok_or_else(|| format!("unknown role `{role}` (use read, run or admin)"))?;
            let d = digest(token);
            if tokens.iter().any(|(t, _)| ct_eq(t, &d)) {
                return Err("duplicate token".to_string());
            }
            tokens.push((d, role));
        }
        if tokens.is_empty() {
            return Err("no tokens configured".to_string());
        }
        Ok(Auth { tokens })
    }

    pub fn authenticate(&self, header: Option<&str>) -> Option<Role> {
        let presented = digest(header?.strip_prefix("Bearer ")?.trim());
        let mut found = None;
        for (stored, role) in &self.tokens {
            // No early exit: timing must not reveal which token matched.
            if ct_eq(stored, &presented) {
                found = Some(*role);
            }
        }
        found
    }
}

/// The authenticated caller of a request.
pub struct Caller {
    pub role: Role,
    /// Who this caller is for per-caller limits: the presented token (as a digest prefix), or `open` without tokens.
    pub key: String,
}

impl Caller {
    pub fn need(&self, needed: Role) -> Result<(), ApiError> {
        if self.role.allows(needed) {
            Ok(())
        } else {
            Err(ApiError::forbidden(needed.as_str()))
        }
    }
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if state.auth.is_open() {
            return Ok(Caller {
                role: Role::Admin,
                key: "open".to_string(),
            });
        }
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let role = state
            .auth
            .authenticate(header)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Caller {
            role,
            key: format!("t:{}", crate::digest_prefix(header.unwrap_or_default())),
        })
    }
}
