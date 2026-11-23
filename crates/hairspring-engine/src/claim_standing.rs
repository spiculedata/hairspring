//! Which backends may back a claim about a run.
//!
//! A backend whose results have not been shown to hold is kept out of every generated claim, whatever its runs found and
//! whoever reported them. The answer lives here, inside the engine: it is a yes or no, not a label, and it is not part of
//! what any listing, API or tool says about a backend. Every backend that ships today may back claims, which is what the
//! claim gate allowed before this existed.

/// Backends whose runs may not back a claim.
const NOT_YET: [&str; 1] = ["dhyve"];

/// Whether the runs of the backend with this id may back a claim. An id nothing here names may, as before.
pub fn backend_may_back_claims(backend_id: &str) -> bool {
    !NOT_YET.contains(&backend_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_core::point::BackendKind;

    #[test]
    fn every_shipped_backend_may_back_claims() {
        for kind in BackendKind::ALL {
            assert!(backend_may_back_claims(kind.as_str()), "{}", kind.as_str());
        }
    }

    #[test]
    fn the_experimental_backend_may_not() {
        assert!(!backend_may_back_claims("dhyve"));
    }

    #[test]
    fn an_id_that_is_not_named_keeps_todays_answer_and_is_matched_exactly() {
        for id in ["", "other", "dhyve2", "DHYVE", " dhyve"] {
            assert!(backend_may_back_claims(id), "{id:?}");
        }
    }
}
