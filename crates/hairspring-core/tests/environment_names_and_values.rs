//! Characterisation of the names and values an environment field accepts. Written against the single global list before
//! the per-backend registry; the registry must keep every spelling and every value grammar exactly as they were.
use hairspring_core::environment::{EnvError, EnvironmentIdentity, MAX_FIELD_VALUE_CHARS};

const NAMES: [&str; 18] = [
    "host.kernel_release",
    "host.cpu_model",
    "host.cpu_flags.sha256",
    "host.container",
    "host.loader.ld_so_cache.sha256",
    "host.loader.libc.sha256",
    "host.loader.ld_so.sha256",
    "host.loader.ld_so_preload.sha256",
    "hermit.path.sha256",
    "hermit.version",
    "env.base_env",
    "env.guest.names",
    "env.guest.names.sha256",
    "env.guest.allowlisted.sha256",
    "env.guest.passed",
    "target.exe.sha256",
    "target.command.sha256",
    "target.cwd.sha256",
];

fn accepts(name: &str, value: &str) -> bool {
    EnvironmentIdentity::default()
        .with_field(name, value)
        .is_ok()
}

#[test]
fn every_name_is_accepted_with_a_value_of_its_own_grammar() {
    for name in NAMES {
        let value = match name {
            "env.guest.passed" => "none",
            n if n.ends_with(".sha256") => "0123456789abcdef",
            _ => "a printable value",
        };
        assert!(accepts(name, value), "{name}");
    }
}

#[test]
fn hash_fields_take_only_lowercase_hex() {
    for name in NAMES.iter().filter(|n| n.ends_with(".sha256")) {
        for bad in ["", "ABC", "xyz", "abc def", "absent-ish"] {
            assert_eq!(
                EnvironmentIdentity::default().with_field(*name, bad),
                Err(EnvError::BadValue(name.to_string())),
                "{name} {bad:?}"
            );
        }
    }
}

#[test]
fn only_the_preload_hash_may_be_the_word_absent() {
    for name in NAMES.iter().filter(|n| n.ends_with(".sha256")) {
        let expected = *name == "host.loader.ld_so_preload.sha256";
        assert_eq!(accepts(name, "absent"), expected, "{name}");
    }
}

#[test]
fn text_fields_take_printable_ascii_up_to_the_limit() {
    for name in NAMES
        .iter()
        .filter(|n| !n.ends_with(".sha256") && **n != "env.guest.passed")
    {
        assert!(accepts(name, &"x".repeat(MAX_FIELD_VALUE_CHARS)), "{name}");
        assert!(
            !accepts(name, &"x".repeat(MAX_FIELD_VALUE_CHARS + 1)),
            "{name}"
        );
        assert!(!accepts(name, "line\nbreak"), "{name}");
        assert!(!accepts(name, "caf\u{e9}"), "{name}");
        assert!(accepts(name, ""), "{name}");
    }
}

#[test]
fn any_other_name_is_refused() {
    for name in ["hermit.other", "host.future", "qemu.version", "env", "x"] {
        assert_eq!(
            EnvironmentIdentity::default().with_field(name, "v"),
            Err(EnvError::UnknownField(name.to_string())),
            "{name}"
        );
    }
}
