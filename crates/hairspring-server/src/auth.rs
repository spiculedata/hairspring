use crate::error::ApiError;
use crate::token_label;
use crate::AppState;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use hairspring_service::Role;
use sha2::{Digest, Sha256};

const MIN_TOKEN_LEN: usize = 16;

/// One configured token: its digest, its role and the operator's label for it, if any.
#[derive(Clone)]
struct Entry {
    digest: [u8; 32],
    role: Role,
    label: Option<String>,
}

/// Bearer-token authentication. Tokens are kept only as SHA-256 digests and compared
/// in constant time across every configured token.
#[derive(Clone, Default)]
pub struct Auth {
    tokens: Vec<Entry>,
}

/// A configuration item before it is checked against the others: the plain token is only held while parsing.
struct Parsed {
    token: String,
    role: Role,
    label: Option<String>,
}

fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `token:role` or `token:role label`. The label starts after the first ASCII space. A token holds no whitespace, so
/// the split cannot fall inside one, and a label holds no `:` or `,`, so the role is still what follows the last `:`.
fn parse_item(item: &str) -> Result<Parsed, String> {
    // Exactly one space separates the label, and the label is not trimmed: a second space, a tab or another kind
    // of whitespace makes the label invalid, as it made the role invalid before labels existed.
    let (head, label) = match item.split_once(' ') {
        Some((head, label)) => (head, Some(label)),
        None => (item, None),
    };
    let (token, role) = head
        .rsplit_once(':')
        .ok_or("each token needs a role: token:role")?;
    if token.len() < MIN_TOKEN_LEN || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!(
            "tokens must be at least {MIN_TOKEN_LEN} characters with no whitespace"
        ));
    }
    // The text in the role slot can be the tail of a token that holds a colon, so it is not repeated.
    let role = Role::parse(role).ok_or("unknown role (use read, run or admin)")?;
    if let Some(label) = label {
        token_label::check_shape(label)?;
    }
    Ok(Parsed {
        token: token.to_string(),
        role,
        label: label.map(str::to_string),
    })
}

/// The checks that need the whole list: no repeated token, no repeated label, no label that holds a token.
fn check_together(parsed: &[Parsed]) -> Result<(), String> {
    let digests: Vec<[u8; 32]> = parsed.iter().map(|p| digest(&p.token)).collect();
    for (i, d) in digests.iter().enumerate() {
        if digests[..i].iter().any(|earlier| ct_eq(earlier, d)) {
            return Err("duplicate token".to_string());
        }
    }
    let mut seen: Vec<String> = Vec::new();
    for (n, p) in parsed.iter().enumerate() {
        let Some(label) = &p.label else { continue };
        if token_label::holds_a_token(label, parsed.iter().map(|q| q.token.as_str())) {
            return Err(format!(
                "entry {}: the label holds a token; labels are not secret, so they must never hold one",
                n + 1
            ));
        }
        let form = token_label::comparison_form(label);
        if seen.contains(&form) {
            return Err(format!(
                "entry {}: the label repeats an earlier label (labels must differ, ignoring case)",
                n + 1
            ));
        }
        seen.push(form);
    }
    Ok(())
}

impl Auth {
    /// No authentication (loopback development only; `start` enforces that).
    pub fn open() -> Auth {
        Auth::default()
    }

    pub fn is_open(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Parses `token:role,token:role label`. Tokens must be at least 16 characters. A label is optional and set only
    /// here (see [`token_label`]). An error names the entry by position (entries are counted, not file lines: a
    /// line with commas is several entries) and repeats neither a token, a label nor the text in the role slot.
    pub fn from_spec(spec: &str) -> Result<Auth, String> {
        let mut parsed: Vec<Parsed> = Vec::new();
        for (n, item) in spec
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .enumerate()
        {
            parsed.push(parse_item(item).map_err(|e| format!("entry {}: {e}", n + 1))?);
        }
        if parsed.is_empty() {
            return Err("no tokens configured".to_string());
        }
        check_together(&parsed)?;
        let tokens = parsed
            .into_iter()
            .map(|p| Entry {
                digest: digest(&p.token),
                role: p.role,
                label: p.label,
            })
            .collect();
        Ok(Auth { tokens })
    }

    pub fn authenticate(&self, header: Option<&str>) -> Option<Role> {
        self.identify(header).map(|(role, _)| role)
    }

    /// The role of the presented token and a key naming that token for per-caller limits. Every header spelling that
    /// authenticates as one token (extra spaces, tabs) gets the same key, so a limit cannot be dodged or flooded by
    /// varying the whitespace.
    pub fn identify(&self, header: Option<&str>) -> Option<(Role, String)> {
        self.identify_with_label(header)
            .map(|(role, key, _)| (role, key))
    }

    /// [`Auth::identify`] and the operator's label for the presented token, if it has one.
    pub fn identify_with_label(
        &self,
        header: Option<&str>,
    ) -> Option<(Role, String, Option<String>)> {
        let raw = header?;
        let (scheme, rest) = raw.split_once([' ', '\t'])?;
        // RFC 7235: the scheme name is case-insensitive.
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let token = rest.trim();
        let presented = digest(token);
        let key = format!("t:{}", crate::digest_prefix(token));
        let mut found: Option<&Entry> = None;
        for entry in &self.tokens {
            // No early exit: timing must not reveal which token matched.
            if ct_eq(&entry.digest, &presented) {
                found = Some(entry);
            }
        }
        found.map(|e| (e.role, key, e.label.clone()))
    }
}

/// The authenticated caller of a request.
pub struct Caller {
    pub role: Role,
    /// Who this caller is for per-caller limits: the presented token (as a digest prefix), or `open` without tokens.
    pub key: String,
    /// The operator's label for this caller's token, from the token configuration only; `None` for an unlabelled
    /// token and in open mode. Never taken from a request.
    pub label: Option<String>,
}

impl Caller {
    pub fn need(&self, needed: Role) -> Result<(), ApiError> {
        if self.role.allows(needed) {
            Ok(())
        } else {
            Err(ApiError::forbidden(needed.as_str()))
        }
    }

    /// Who to name in a log line: the label and the key, or the key alone. The label is validated when the server
    /// starts and is passed through `public_text` here as well. The token is never part of it.
    pub fn who(&self) -> String {
        match &self.label {
            Some(label) => format!(
                "{} ({})",
                hairspring_service::sanitize::public_text(label),
                self.key
            ),
            None => self.key.clone(),
        }
    }

    /// The log line for an action by this caller.
    pub fn action_line(&self, action: &str) -> String {
        format!("hairspring-server: {action} by {}", self.who())
    }

    /// Records an action by this caller on standard error, as one line.
    pub fn log_action(&self, action: &str) {
        eprintln!("{}", self.action_line(action));
    }
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if state.auth.is_open() {
            return Ok(Caller {
                role: Role::Admin,
                key: "open".to_string(),
                label: None,
            });
        }
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let (role, key, label) = state
            .auth
            .identify_with_label(header)
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Caller { role, key, label })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "run-token-0123456789abcdef";
    const OTHER: &str = "other-token-0123456789abc";

    fn auth() -> Auth {
        Auth::from_spec(&format!("{TOKEN}:run,{OTHER}:read")).unwrap()
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
            label: None,
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

    // Labels.

    fn labelled() -> Auth {
        Auth::from_spec(&format!("{TOKEN}:run ci bot, {OTHER}:read")).unwrap()
    }

    #[test]
    fn a_label_follows_the_role_after_a_space_and_is_found_for_its_own_token_only() {
        let a = labelled();
        let (role, _, label) = a
            .identify_with_label(Some(&format!("Bearer {TOKEN}")))
            .unwrap();
        assert_eq!((role, label.as_deref()), (Role::Run, Some("ci bot")));
        let (_, _, none) = a
            .identify_with_label(Some(&format!("Bearer {OTHER}")))
            .unwrap();
        assert_eq!(none, None);
    }

    #[test]
    fn every_old_form_parses_exactly_as_before_and_has_no_label() {
        let colon_token = "tok:with:colons-0123456789";
        for (spec, role) in [
            (format!("{TOKEN}:read"), Role::Read),
            (format!("  {TOKEN}:admin  "), Role::Admin),
            (format!("{TOKEN}:run,{OTHER}:read"), Role::Run),
            (format!("{TOKEN}:run, ,{OTHER}:read,"), Role::Run),
            (format!("{colon_token}:run"), Role::Run),
        ] {
            let a = Auth::from_spec(&spec).unwrap();
            let first = spec
                .split(',')
                .map(str::trim)
                .find(|s| !s.is_empty())
                .unwrap();
            let token = first.rsplit_once(':').unwrap().0;
            let (got, key, label) = a
                .identify_with_label(Some(&format!("Bearer {token}")))
                .unwrap();
            assert_eq!((got, label), (role, None), "{spec:?}");
            assert_eq!(key, format!("t:{}", crate::digest_prefix(token)));
        }
    }

    #[test]
    fn a_spec_that_was_refused_before_is_still_refused() {
        for bad in [
            TOKEN.to_string(),
            format!("{TOKEN}:owner"),
            format!("{TOKEN}:read:"),
            "short:read".to_string(),
            format!("{TOKEN}:read\textra"),
            format!("{TOKEN}:read\u{a0}x"),
            format!("a b {TOKEN}:read"),
            format!("{TOKEN}: read"),
        ] {
            assert!(Auth::from_spec(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_bad_label_refuses_the_whole_spec_and_the_message_repeats_neither_token_nor_label() {
        for label in [
            "bell\u{7}",
            "rlo\u{202e}x",
            "ls\u{2028}x",
            "caf\u{e9}",
            "a:b",
            "a=b",
            &"x".repeat(token_label::MAX_LABEL_CHARS + 1),
        ] {
            let err = Auth::from_spec(&format!("{TOKEN}:run {label}"))
                .err()
                .unwrap_or_else(|| panic!("{label:?} accepted"));
            assert!(!err.contains(TOKEN), "{err}");
            assert!(!err.contains(label), "{err}");
            assert!(err.starts_with("entry 1:"), "{err}");
        }
    }

    #[test]
    fn extra_or_unusual_whitespace_before_a_label_is_refused_as_it_was_before_labels() {
        for gap in [" ", "\t", "\u{2028}", "\u{a0}", " \t", "\u{200b}"] {
            let spec = format!("{TOKEN}:run {gap}ci");
            assert!(Auth::from_spec(&spec).is_err(), "{spec:?}");
        }
        assert!(Auth::from_spec(&format!("{TOKEN}:run ci")).is_ok());
    }

    #[test]
    fn an_unknown_role_message_does_not_echo_the_text_in_the_role_slot() {
        // A token with a colon: its tail lands in the role slot when the role is missing or wrong.
        let colon_token = "abcdefghijklmnopqr:SECRETTAIL-0123456789";
        for spec in [
            colon_token.to_string(),
            format!("{TOKEN}:ro\u{202e}le"),
            format!("{TOKEN}:bogus"),
        ] {
            let err = Auth::from_spec(&spec).err().expect("refused");
            assert!(
                !err.contains("SECRETTAIL") && !err.contains("bogus") && !err.contains('\u{202e}'),
                "{err}"
            );
        }
        let err = Auth::from_spec(colon_token).err().unwrap();
        assert_eq!(err, "entry 1: unknown role (use read, run or admin)");
    }

    #[test]
    fn an_entry_number_counts_entries_not_file_lines() {
        let err = Auth::from_spec(&format!("{TOKEN}:run,,{OTHER}:bogus"))
            .err()
            .unwrap();
        assert!(err.starts_with("entry 2:"), "{err}");
    }

    #[test]
    fn a_comma_ends_the_entry_so_it_cannot_be_part_of_a_label() {
        assert!(Auth::from_spec(&format!("{TOKEN}:run a,b")).is_err());
    }

    #[test]
    fn a_label_equal_to_or_holding_any_configured_token_is_refused() {
        for spec in [
            format!("{TOKEN}:run {OTHER},{OTHER}:read"),
            format!("{TOKEN}:run {TOKEN}"),
            format!("{OTHER}:read a-{TOKEN}-b,{TOKEN}:run"),
            format!("{TOKEN}:run {}", TOKEN.to_ascii_uppercase()),
        ] {
            let err = Auth::from_spec(&spec).err().expect("refused");
            assert!(err.contains("holds a token"), "{err}");
            assert!(!err.contains(TOKEN) && !err.contains(OTHER), "{err}");
        }
    }

    #[test]
    fn labels_must_differ_ignoring_case_and_a_repeated_token_is_still_refused() {
        assert!(Auth::from_spec(&format!("{TOKEN}:run ci,{OTHER}:read CI")).is_err());
        assert!(Auth::from_spec(&format!("{TOKEN}:run ci,{OTHER}:read ci2")).is_ok());
        let err = Auth::from_spec(&format!("{TOKEN}:run ci,{TOKEN}:read other"))
            .err()
            .expect("refused");
        assert_eq!(err, "duplicate token");
    }

    #[test]
    fn who_names_the_label_and_key_or_the_key_alone_and_never_the_token() {
        let caller = |label: Option<&str>| Caller {
            role: Role::Run,
            key: "t:0011223344556677".into(),
            label: label.map(str::to_string),
        };
        assert_eq!(caller(Some("ci")).who(), "ci (t:0011223344556677)");
        assert_eq!(caller(None).who(), "t:0011223344556677");
        let line = caller(Some("ci")).action_line("run created");
        assert_eq!(
            line,
            "hairspring-server: run created by ci (t:0011223344556677)"
        );
        // Defence in depth: a label with a control character cannot start a second log line.
        let forged = caller(Some("a\nforged line")).action_line("replay started");
        assert!(!forged.contains('\n'), "{forged:?}");
    }

    #[test]
    fn a_log_line_built_from_an_authenticated_caller_never_holds_the_token() {
        let a = labelled();
        let (role, key, label) = a
            .identify_with_label(Some(&format!("Bearer {TOKEN}")))
            .unwrap();
        let line = Caller { role, key, label }.action_line("worker registered");
        assert!(!line.contains(TOKEN), "{line}");
        assert!(line.contains("ci bot"), "{line}");
    }
}
