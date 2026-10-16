//! Canonical forms the identity enforces: a value in another spelling is rejected, never hashed.
mod point_common;

use hairspring_core::point::{HermitInputs, IdentityDraft, IdentityError};
use point_common::*;

fn finish(change: impl FnOnce(HermitInputs) -> HermitInputs) -> Result<String, IdentityError> {
    let draft = IdentityDraft::Hermit(common("queue"), change(hermit_inputs()));
    draft.finish().map(|i| i.hash())
}

fn is_invalid(result: Result<String, IdentityError>, expected: &str) -> bool {
    matches!(result, Err(IdentityError::Invalid { field, .. }) if field == expected)
}

#[test]
fn epoch_must_be_rfc_3339_in_utc_with_z() {
    let ok = ["2026-01-01T00:00:00Z", "1999-12-31T23:59:59Z"];
    for epoch in ok {
        assert!(
            finish(|h| HermitInputs {
                epoch: v(epoch.to_string()),
                ..h
            })
            .is_ok(),
            "{epoch}"
        );
    }
    let bad = [
        "2026-01-01T00:00:00+00:00",
        "2026-01-01T00:00:00-00:00",
        "2026-01-01T00:00:00z",
        "2026-01-01t00:00:00Z",
        "2026-01-01 00:00:00Z",
        "2026-01-01T00:00:00.000Z",
        "2026-01-01T00:00:00",
        "2026-01-01T00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-01-32T00:00:00Z",
        "2026-01-00T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
        "2026-1-1T0:0:0Z",
        " 2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z ",
        "+2026-01-01T00:00:00Z",
    ];
    for epoch in bad {
        let result = finish(|h| HermitInputs {
            epoch: v(epoch.to_string()),
            ..h
        });
        assert!(is_invalid(result, "epoch"), "{epoch:?}");
    }
}

#[test]
fn cwd_must_be_an_absolute_resolved_path() {
    for cwd in ["/", "/work", "/a/b c/d", "/with.dot/x.y"] {
        assert!(
            finish(|h| HermitInputs {
                cwd: v(cwd.to_string()),
                ..h
            })
            .is_ok(),
            "{cwd}"
        );
    }
    let bad = [
        "work",
        "./work",
        "../work",
        "/work/",
        "/work//x",
        "//work",
        "/work/./x",
        "/work/../x",
        "/..",
        "/.",
        "~/work",
    ];
    for cwd in bad {
        let result = finish(|h| HermitInputs {
            cwd: v(cwd.to_string()),
            ..h
        });
        assert!(is_invalid(result, "cwd"), "{cwd:?}");
    }
}

#[test]
fn events_transport_must_be_one_of_the_known_values() {
    for transport in ["file", "stream"] {
        let result = finish(|h| HermitInputs {
            events_transport: v(transport.to_string()),
            ..h
        });
        assert!(result.is_ok(), "{transport}");
    }
    for transport in ["File", "pipe", "file ", "stream\n", "none"] {
        let result = finish(|h| HermitInputs {
            events_transport: v(transport.to_string()),
            ..h
        });
        assert!(is_invalid(result, "events_transport"), "{transport:?}");
    }
}

#[test]
fn an_unset_timeout_and_a_zero_timeout_differ() {
    let none = finish(|h| HermitInputs {
        trial_timeout_secs: v(None),
        ..h
    })
    .unwrap();
    let zero = finish(|h| HermitInputs {
        trial_timeout_secs: v(Some(0)),
        ..h
    })
    .unwrap();
    assert_ne!(none, zero);
}
