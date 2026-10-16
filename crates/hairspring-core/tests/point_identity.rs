mod point_common;

use hairspring_core::point::{
    BackendKind, CommonInputs, HermitInputs, IdentityDraft, IdentityError, Recorded, TmpPolicy,
    WasmInputs,
};
use point_common::*;

#[test]
fn golden_identity_text_and_hash_for_each_backend() {
    let cases = [
        (sim_draft(), BackendKind::Sim, SIM_TEXT, SIM_IDENTITY_HASH),
        (
            wasm_draft(),
            BackendKind::Wasm,
            WASM_TEXT,
            WASM_IDENTITY_HASH,
        ),
        (
            hermit_draft(),
            BackendKind::Hermit,
            HERMIT_TEXT,
            HERMIT_IDENTITY_HASH,
        ),
    ];
    for (draft, backend, text, hash) in cases {
        let identity = draft.finish().unwrap();
        assert_eq!(identity.backend(), backend);
        assert_eq!(identity.text(), text);
        assert_eq!(identity.hash(), hash);
    }
}

#[test]
fn a_run_missing_any_input_has_no_identity() {
    let none = CommonInputs::default();
    assert_eq!(
        IdentityDraft::Sim(none.clone()).finish(),
        Err(IdentityError::Incomplete {
            missing: vec!["hairspring_sha", "target"]
        })
    );
    let wasm = IdentityDraft::Wasm(none, WasmInputs::default());
    assert_eq!(
        wasm.finish(),
        Err(IdentityError::Incomplete {
            missing: vec![
                "hairspring_sha",
                "target",
                "module_sha",
                "fuel",
                "max_memory_bytes",
                "vm_semantics"
            ]
        })
    );
    let hermit = IdentityDraft::Hermit(common("queue"), HermitInputs::default());
    let Err(IdentityError::Incomplete { missing }) = hermit.finish() else {
        panic!("an empty hermit record must be incomplete");
    };
    assert_eq!(missing.len(), 19);
    assert_eq!(missing.first(), Some(&"hermit_sha"));
    assert_eq!(missing.last(), Some(&"hang_is_violation"));
}

#[test]
fn each_single_missing_wasm_input_is_reported_by_name() {
    let base = wasm_inputs();
    let cases = [
        (
            "module_sha",
            WasmInputs {
                module_sha: Recorded::Missing,
                ..base.clone()
            },
        ),
        (
            "fuel",
            WasmInputs {
                fuel: Recorded::Missing,
                ..base.clone()
            },
        ),
        (
            "max_memory_bytes",
            WasmInputs {
                max_memory_bytes: Recorded::Missing,
                ..base.clone()
            },
        ),
        (
            "vm_semantics",
            WasmInputs {
                vm_semantics: Recorded::Missing,
                ..base.clone()
            },
        ),
    ];
    for (name, inputs) in cases {
        let result = IdentityDraft::Wasm(common("counter"), inputs).finish();
        assert_eq!(
            result,
            Err(IdentityError::Incomplete {
                missing: vec![name]
            })
        );
    }
}

#[test]
fn a_missing_common_input_makes_every_backend_incomplete() {
    let no_sha = CommonInputs {
        hairspring_sha: Recorded::Missing,
        ..common("t")
    };
    let expected = Err(IdentityError::Incomplete {
        missing: vec!["hairspring_sha"],
    });
    assert_eq!(IdentityDraft::Sim(no_sha.clone()).finish(), expected);
    assert_eq!(
        IdentityDraft::Wasm(no_sha.clone(), wasm_inputs()).finish(),
        expected
    );
    assert_eq!(
        IdentityDraft::Hermit(no_sha, hermit_inputs()).finish(),
        expected
    );
}

#[test]
fn values_that_are_not_recorded_content_are_treated_as_missing() {
    let bad_shas = ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65)];
    for bad in bad_shas {
        let draft = IdentityDraft::Sim(CommonInputs {
            hairspring_sha: v(bad.to_string()),
            ..common("t")
        });
        assert_eq!(
            draft.finish(),
            Err(IdentityError::Incomplete {
                missing: vec!["hairspring_sha"]
            }),
            "sha {bad:?}"
        );
    }
    let no_name = IdentityDraft::Sim(common(""));
    assert_eq!(
        no_name.finish(),
        Err(IdentityError::Incomplete {
            missing: vec!["target"]
        })
    );
    let no_argv = HermitInputs {
        target_argv: v(vec![]),
        ..hermit_inputs()
    };
    assert_eq!(
        IdentityDraft::Hermit(common("q"), no_argv).finish(),
        Err(IdentityError::Incomplete {
            missing: vec!["target_argv"]
        })
    );
}

#[test]
fn hex_case_of_a_content_hash_does_not_change_the_identity() {
    let upper = CommonInputs {
        hairspring_sha: v(sha('a').to_uppercase()),
        ..common("lock_demo")
    };
    let identity = IdentityDraft::Sim(upper).finish().unwrap();
    assert_eq!(identity.text(), SIM_TEXT);
    assert_eq!(identity.hash(), SIM_IDENTITY_HASH);
}

#[test]
fn the_same_common_inputs_under_different_backends_never_share_an_identity() {
    let sim = IdentityDraft::Sim(common("t")).finish().unwrap();
    let wasm = IdentityDraft::Wasm(common("t"), wasm_inputs())
        .finish()
        .unwrap();
    let hermit = IdentityDraft::Hermit(common("t"), hermit_inputs())
        .finish()
        .unwrap();
    assert_ne!(sim.hash(), wasm.hash());
    assert_ne!(sim.hash(), hermit.hash());
    assert_ne!(wasm.hash(), hermit.hash());
}

fn hermit_with(change: impl FnOnce(HermitInputs) -> HermitInputs) -> Result<String, IdentityError> {
    let draft = IdentityDraft::Hermit(common("queue"), change(hermit_inputs()));
    draft.finish().map(|i| i.hash())
}

#[test]
fn list_items_cannot_merge_or_split() {
    let split = hermit_with(|h| HermitInputs {
        hermit_args: v(strings(&["a", "b"])),
        ..h
    })
    .unwrap();
    let joined = hermit_with(|h| HermitInputs {
        hermit_args: v(strings(&["a,b"])),
        ..h
    })
    .unwrap();
    let escaped = hermit_with(|h| HermitInputs {
        hermit_args: v(strings(&["a%2Cb"])),
        ..h
    })
    .unwrap();
    assert_ne!(split, joined);
    assert_ne!(joined, escaped);
    let none = hermit_with(|h| HermitInputs {
        hermit_args: v(strings(&[])),
        ..h
    })
    .unwrap();
    let blank = hermit_with(|h| HermitInputs {
        hermit_args: v(strings(&[""])),
        ..h
    })
    .unwrap();
    assert_ne!(none, blank);
}

#[test]
fn a_value_cannot_start_a_new_line() {
    let injected = "x\nhang_is_violation=true";
    let draft = IdentityDraft::Hermit(
        common("queue"),
        HermitInputs {
            cwd: v(injected.to_string()),
            ..hermit_inputs()
        },
    );
    let identity = draft.finish().unwrap();
    assert!(identity.text().contains("cwd=x%0Ahang_is_violation=true\n"));
    let own_lines = identity
        .text()
        .lines()
        .filter(|l| l.starts_with("hang_is_violation="));
    assert_eq!(own_lines.count(), 1);
    assert!(identity.text().ends_with("hang_is_violation=false\n"));
    let carriage = IdentityDraft::Sim(common("a\rb%")).finish().unwrap();
    assert!(carriage.text().contains("target=a%0Db%25\n"));
}

#[test]
fn guest_env_order_is_part_of_the_identity() {
    let forward = vec![
        ("A".to_string(), "1".to_string()),
        ("B".to_string(), "2".to_string()),
    ];
    let backward = vec![
        ("B".to_string(), "2".to_string()),
        ("A".to_string(), "1".to_string()),
    ];
    let a = hermit_with(|h| HermitInputs {
        guest_env: v(forward),
        ..h
    })
    .unwrap();
    let b = hermit_with(|h| HermitInputs {
        guest_env: v(backward),
        ..h
    })
    .unwrap();
    assert_ne!(a, b);
}

#[test]
fn guest_env_entries_are_validated() {
    let bad_envs = [
        vec![("HAIRSPRING_SEED".to_string(), "1".to_string())],
        vec![
            ("A".to_string(), "1".to_string()),
            ("A".to_string(), "2".to_string()),
        ],
        vec![("A=B".to_string(), "1".to_string())],
        vec![(String::new(), "1".to_string())],
    ];
    for env in bad_envs {
        let result = hermit_with(|h| HermitInputs {
            guest_env: v(env.clone()),
            ..h
        });
        assert!(
            matches!(
                result,
                Err(IdentityError::Invalid {
                    field: "guest_env",
                    ..
                })
            ),
            "{env:?}"
        );
    }
    let empty = hermit_with(|h| HermitInputs {
        guest_env: v(vec![]),
        ..h
    });
    assert!(empty.is_ok());
}

#[test]
fn tmp_policy_none_and_a_path_named_none_differ() {
    let none = hermit_with(|h| HermitInputs {
        tmp_policy: v(TmpPolicy::None),
        ..h
    })
    .unwrap();
    let named = hermit_with(|h| HermitInputs {
        tmp_policy: v(TmpPolicy::Path("none".to_string())),
        ..h
    });
    assert_ne!(none, named.unwrap());
    let empty = hermit_with(|h| HermitInputs {
        tmp_policy: v(TmpPolicy::Path(String::new())),
        ..h
    });
    assert!(matches!(
        empty,
        Err(IdentityError::Invalid {
            field: "tmp_policy",
            ..
        })
    ));
}

#[test]
fn oversized_identity_text_is_refused() {
    let huge = "x".repeat(20 * 1024);
    let result = IdentityDraft::Sim(common(&huge)).finish();
    assert!(matches!(result, Err(IdentityError::TooLarge { bytes }) if bytes > 16 * 1024));
}

#[test]
fn errors_have_readable_messages() {
    let incomplete = IdentityError::Incomplete {
        missing: vec!["fuel", "target"],
    };
    assert_eq!(
        incomplete.to_string(),
        "identity incomplete: not recorded: fuel, target"
    );
    let invalid = IdentityError::Invalid {
        field: "guest_env",
        reason: "duplicate key",
    };
    assert_eq!(
        invalid.to_string(),
        "identity input `guest_env` is invalid: duplicate key"
    );
    assert_eq!(
        IdentityError::TooLarge { bytes: 20000 }.to_string(),
        "identity text is 20000 bytes, over the 16384 byte limit"
    );
}
