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
        self.identify(header).map(|(role, _)| role)
    }

    /// The role of the presented token and a key naming that token for per-caller limits. Every header spelling that
    /// authenticates as one token (extra spaces, tabs) gets the same key, so a limit cannot be dodged or flooded by
    /// varying the whitespace.
    pub fn identify(&self, header: Option<&str>) -> Option<(Role, String)> {
        let raw = header?;
        let (scheme, rest) = raw.split_once([' ', '\t'])?;
        // RFC 7235: the scheme name is case-insensitive.
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let token = rest.trim();
        let presented = digest(token);
        let key = format!("t:{}", crate::digest_prefix(token));
        let mut found = None;
        for (stored, role) in &self.tokens {
            // No early exit: timing must not reveal which token matched.
            if ct_eq(stored, &presented) {
                found = Some(*role);
            }
        }
        found.map(|role| (role, key))
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
        let (role, key) = state
            .auth
            .identify(header)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Caller { role, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "run-token-0123456789abcdef";

    fn auth() -> Auth {
        Auth::from_spec(&format!("{TOKEN}:run,other-token-0123456789abc:read")).unwrap()
    }

    #[test]
    fn every_spelling_of_one_token_has_one_key() {
        let a = auth();
        let (_, plain) = a.identify(Some(&format!("Bearer {TOKEN}"))).unwrap();
        for variant in [
            format!("Bearer  {TOKEN}"),
            format!("Bearer {TOKEN} "),
            format!("Bearer \t {TOKEN}\t"),
            format!("Bearer      {TOKEN}"),
        ] {
            let (role, key) = a.identify(Some(&variant)).expect("still authenticates");
            assert_eq!(
                (role, key.as_str()),
                (Role::Run, plain.as_str()),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn the_scheme_name_is_case_insensitive_and_keeps_one_key() {
        let a = auth();
        let (_, plain) = a.identify(Some(&format!("Bearer {TOKEN}"))).unwrap();
        for variant in [
            format!("bearer {TOKEN}"),
            format!("BEARER {TOKEN}"),
            format!("bEaReR {TOKEN}"),
            format!("bearer\t{TOKEN}"),
            format!("BEARER \t {TOKEN}  "),
        ] {
            let (role, key) = a.identify(Some(&variant)).expect("authenticates");
            assert_eq!(
                (role, key.as_str()),
                (Role::Run, plain.as_str()),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn odd_spellings_that_are_not_one_scheme_and_one_token_are_refused() {
        let a = auth();
        for variant in [
            format!("Bearer Bearer {TOKEN}"),
            format!("Bearer{TOKEN}"),
            format!("Basic {TOKEN}"),
            format!(" Bearer {TOKEN}"),
            format!("Bearer {TOKEN} extra"),
            "Bearer".to_string(),
            "Bearer ".to_string(),
            "bearer \t ".to_string(),
            String::new(),
        ] {
            assert!(a.identify(Some(&variant)).is_none(), "{variant:?}");
        }
    }

    #[test]
    fn different_tokens_have_different_keys() {
        let a = auth();
        let (_, one) = a.identify(Some(&format!("Bearer {TOKEN}"))).unwrap();
        let (_, two) = a
            .identify(Some("Bearer other-token-0123456789abc"))
            .unwrap();
        assert_ne!(one, two);
    }

    #[test]
    fn the_comparison_is_false_for_any_pair_of_digests_that_differ() {
        let zero = [0u8; 32];
        assert!(ct_eq(&zero, &zero));
        // Two bytes that differ by the same amount must not cancel each other out.
        let mut two_equal_differences = zero;
        two_equal_differences[0] = 1;
        two_equal_differences[31] = 1;
        assert!(!ct_eq(&zero, &two_equal_differences));
        for i in 0..32 {
            let mut one = zero;
            one[i] = 0x80;
            assert!(!ct_eq(&zero, &one), "byte {i}");
        }
    }

    #[test]
    fn a_token_of_exactly_the_minimum_length_is_accepted_and_one_shorter_is_not() {
        let sixteen = "a".repeat(MIN_TOKEN_LEN);
        assert!(Auth::from_spec(&format!("{sixteen}:read")).is_ok());
        let fifteen = "a".repeat(MIN_TOKEN_LEN - 1);
        assert!(Auth::from_spec(&format!("{fifteen}:read")).is_err());
    }

    #[test]
    fn a_token_with_whitespace_or_a_control_character_is_refused() {
        for bad in [
            "token with spaces 0123456789",
            "token-with-bell-\u{7}-0123456789",
            "token-with-tab-\t-0123456789",
            "token-with-nul-\0-0123456789",
            "token-with-nbsp-\u{a0}-0123456789",
        ] {
            assert!(Auth::from_spec(&format!("{bad}:read")).is_err(), "{bad:?}");
        }
        assert!(Auth::from_spec("token-without-either-0123456789:read").is_ok());
    }

    #[test]
    fn a_spec_without_a_valid_role_or_with_a_repeated_token_or_with_no_token_is_refused() {
        assert!(Auth::from_spec("run-token-0123456789abcdef").is_err());
        assert!(Auth::from_spec("run-token-0123456789abcdef:owner").is_err());
        assert!(Auth::from_spec(&format!("{TOKEN}:read,{TOKEN}:admin")).is_err());
        assert!(Auth::from_spec("").is_err());
        assert!(Auth::from_spec(" , ").is_err());
    }

    #[test]
    fn a_configured_auth_is_not_open_and_the_roles_come_back_as_configured() {
        let a = auth();
        assert!(!a.is_open());
        assert!(Auth::open().is_open());
        assert_eq!(
            a.authenticate(Some(&format!("Bearer {TOKEN}"))),
            Some(Role::Run)
        );
        assert_eq!(
            a.authenticate(Some("Bearer other-token-0123456789abc")),
            Some(Role::Read)
        );
        assert_eq!(
            a.authenticate(Some("Bearer nope-nope-nope-nope-nope")),
            None
        );
        assert_eq!(a.authenticate(None), None);
    }

    #[test]
    fn a_caller_is_refused_a_role_above_its_own_and_allowed_its_own_and_below() {
        let caller = Caller {
            role: Role::Run,
            key: "t:x".into(),
        };
        assert!(caller.need(Role::Read).is_ok());
        assert!(caller.need(Role::Run).is_ok());
        assert!(caller.need(Role::Admin).is_err());
    }

    #[test]
    fn an_unknown_token_has_no_key() {
        assert!(auth()
            .identify(Some("Bearer nope-nope-nope-nope-nope"))
            .is_none());
    }
}
