//! `pass_env` on process targets, and what the target's `env` and `hermit_args` may name.
use hairspring_engine::config::{Config, ConfigError, TargetKind};

fn parse(extra: &str) -> Result<Config, ConfigError> {
    Config::parse(&format!(
        "[target.x]\nkind = \"process\"\ncommand = [\"./svc\"]\n{extra}\n"
    ))
}

fn pass_env(extra: &str) -> Result<bool, ConfigError> {
    match &parse(extra)?.targets["x"].kind {
        TargetKind::Process { pass_env, .. } => Ok(*pass_env),
        other => panic!("expected a process target, got {}", other.name()),
    }
}

fn message(result: Result<Config, ConfigError>) -> String {
    result
        .expect_err("the config should be refused")
        .to_string()
}

#[test]
fn the_engine_passes_its_variables_unless_told_not_to() {
    assert!(pass_env("").unwrap());
    assert!(pass_env("pass_env = true").unwrap());
    assert!(!pass_env("pass_env = false").unwrap());
}

#[test]
fn pass_env_is_refused_on_other_target_kinds() {
    for body in [
        "kind = \"sim\"\nsim = \"lock-lease\"",
        "kind = \"wasm\"\nmodule = \"m.wasm\"",
    ] {
        let text = message(Config::parse(&format!(
            "[target.x]\n{body}\npass_env = false\n"
        )));
        assert!(text.contains("pass_env"), "{text}");
        assert!(text.contains("only valid for kind = \"process\""), "{text}");
    }
}

#[test]
fn a_misspelt_key_is_still_an_error() {
    assert!(matches!(
        parse("pass-env = false"),
        Err(ConfigError::Parse(_))
    ));
}

#[test]
fn the_serialised_target_shows_the_setting() {
    let json = serde_json::to_string(&parse("pass_env = false").unwrap().targets["x"]).unwrap();
    assert!(json.contains("\"pass_env\":false"), "{json}");
}

#[test]
fn an_env_key_hermit_cannot_pass_is_refused_only_when_passing_is_active() {
    let bad = "env = { \"A=B\" = \"1\" }";
    for args in [
        "hermit_args = [\"--base-env=minimal\"]",
        "hermit_args = [\"--base-env=Empty\"]",
    ] {
        let text = message(parse(&format!("{args}\n{bad}")));
        assert!(text.contains("target `x`"), "{text}");
        assert!(text.contains("pass_env = false"), "{text}");
        assert!(
            !text.contains("A=B"),
            "the caller's key is not echoed: {text}"
        );
    }
    assert!(parse(&format!(
        "hermit_args = [\"--base-env=minimal\"]\npass_env = false\n{bad}"
    ))
    .is_ok());
    assert!(parse(bad).is_ok(), "no flag: Hermit passes its environment");
    assert!(parse(&format!("hermit_args = [\"--base-env=host\"]\n{bad}")).is_ok());
    assert!(parse(&format!(
        "hermit_args = [\"--base-env=minimal\", \"--config=f\"]\n{bad}"
    ))
    .is_ok());
    let empty_key = "env = { \"\" = \"1\" }";
    assert!(parse(&format!(
        "hermit_args = [\"--base-env=minimal\"]\n{empty_key}"
    ))
    .is_err());
    let non_ascii = "env = { \"caf\u{e9}\" = \"1\" }";
    assert!(parse(&format!(
        "hermit_args = [\"--base-env=minimal\"]\n{non_ascii}"
    ))
    .is_err());
}

#[test]
fn an_ordinary_env_is_accepted_under_minimal() {
    assert!(parse(
        "hermit_args = [\"--base-env=minimal\"]\nenv = { GOMAXPROCS = \"2\", TMPDIR = \"/x\" }"
    )
    .is_ok());
}

#[test]
fn the_target_cannot_name_a_variable_the_engine_owns_in_env() {
    for name in [
        "HAIRSPRING_PROC",
        "HAIRSPRING_PROC_PID",
        "HAIRSPRING_EVENTS_FILE",
    ] {
        for args in ["", "hermit_args = [\"--base-env=minimal\"]"] {
            let text = message(parse(&format!("{args}\nenv = {{ {name} = \"x\" }}")));
            assert!(text.contains("target `x`"), "{text}");
            assert!(text.contains(name), "{text}");
            assert!(text.contains("Hairspring sets it"), "{text}");
        }
    }
}

#[test]
fn the_target_cannot_name_a_variable_the_engine_owns_in_hermit_args() {
    for name in [
        "HAIRSPRING_PROC",
        "HAIRSPRING_PROC_PID",
        "HAIRSPRING_EVENTS_FILE",
    ] {
        for form in [
            format!("[\"--env={name}=x\"]"),
            format!("[\"--env\", \"{name}\"]"),
            format!("[\"-e\", \"{name}=x\"]"),
            format!("[\"-e{name}\"]"),
            format!("[\"-e={name}\"]"),
        ] {
            let text = message(parse(&format!("hermit_args = {form}")));
            assert!(text.contains(name), "{form}: {text}");
            assert!(text.contains("Hairspring sets it"), "{form}: {text}");
        }
    }
}

#[test]
fn the_seed_variable_and_ordinary_names_are_not_refused() {
    assert!(
        parse("hermit_args = [\"--env=HAIRSPRING_SEED=1\", \"--env=GOTRACEBACK=all\"]").is_ok()
    );
    assert!(parse("env = { HAIRSPRING_SEED = \"1\", HAIRSPRING_EVENTS = \"1\" }").is_ok());
}
