//! What the backend records about the host, read from a fake root so the tests do not depend on the machine.
mod common;
use common::scratch;
use hairspring_core::environment::EnvironmentIdentity;
use hairspring_hermit::host_identity::{collect, GuestInputs, HermitIdentity, HostSources};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CPUINFO: &str = "processor\t: 0\nmodel name\t: Fake CPU @ 3.0GHz\nflags\t\t: sse2 avx2 rdrand\n\nprocessor\t: 1\nmodel name\t: Fake CPU @ 3.0GHz\nflags\t\t: sse2 avx2 rdrand\n";
const HERMIT_SHA: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn fake_host(name: &str) -> PathBuf {
    let root = scratch(name);
    put(&root, "proc/sys/kernel/osrelease", "6.8.0-45-generic\n");
    put(&root, "proc/cpuinfo", CPUINFO);
    put(&root, "etc/ld.so.cache", "cache-one");
    put(&root, "lib/x86_64-linux-gnu/libc.so.6", "libc-one");
    put(&root, "lib64/ld-linux-x86-64.so.2", "ld-one");
    root
}

fn hermit() -> HermitIdentity {
    HermitIdentity {
        sha256: Some(HERMIT_SHA.to_string()),
        version: Some("hermit 0.1.0".to_string()),
    }
}

struct Inputs {
    command: Vec<String>,
    env: BTreeMap<String, String>,
    extra: Vec<String>,
    args: Vec<String>,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            command: vec!["/bin/sh".to_string()],
            env: BTreeMap::from([("PATH".to_string(), "/bin".to_string())]),
            extra: vec![],
            args: vec![],
        }
    }
}

fn run(root: &Path, hermit: &HermitIdentity, i: &Inputs) -> EnvironmentIdentity {
    collect(
        &HostSources::rooted(root),
        hermit,
        &GuestInputs {
            command: &i.command,
            cwd: None,
            env: &i.env,
            extra_env_names: &i.extra,
            hermit_args: &i.args,
        },
    )
}

fn identity(root: &Path) -> EnvironmentIdentity {
    run(root, &hermit(), &Inputs::default())
}

#[test]
fn every_documented_field_is_populated() {
    let id = identity(&fake_host("populated"));
    assert_eq!(id.get("host.kernel_release"), Some("6.8.0-45-generic"));
    assert_eq!(id.get("host.cpu_model"), Some("Fake CPU @ 3.0GHz"));
    for name in [
        "host.cpu_flags.sha256",
        "host.loader.ld_so_cache.sha256",
        "host.loader.libc.sha256",
        "host.loader.ld_so.sha256",
        "env.guest.allowlisted.sha256",
        "target.exe.sha256",
    ] {
        let value = id.get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(value.len(), 64, "{name}: {value}");
    }
    assert_eq!(id.get("hermit.path.sha256"), Some(HERMIT_SHA));
    assert_eq!(id.get("hermit.version"), Some("hermit 0.1.0"));
    assert_eq!(id.get("env.guest.names"), Some("PATH"));
    assert_eq!(id.get("host.container"), Some("none"));
    assert_eq!(id.get("host.loader.ld_so_preload.sha256"), Some("absent"));
}

#[test]
fn the_same_host_gives_the_same_identity() {
    let root = fake_host("stable");
    assert_eq!(identity(&root), identity(&root));
}

#[test]
fn a_rewritten_loader_cache_changes_only_its_hash() {
    let root = fake_host("cache");
    let before = identity(&root);
    put(&root, "etc/ld.so.cache", "cache-two");
    let after = identity(&root);
    let changed: Vec<&str> = before
        .fields()
        .filter(|(k, v)| after.get(k) != Some(v))
        .map(|(k, _)| k)
        .collect();
    assert_eq!(changed, ["host.loader.ld_so_cache.sha256"]);
}

#[test]
fn a_preload_file_changes_the_loader_identity() {
    let root = fake_host("preload");
    let before = identity(&root);
    put(&root, "etc/ld.so.preload", "/opt/libhook.so\n");
    let after = identity(&root);
    assert_eq!(
        before.get("host.loader.ld_so_preload.sha256"),
        Some("absent")
    );
    let hash = after.get("host.loader.ld_so_preload.sha256").unwrap();
    assert_eq!(hash.len(), 64);
    assert!(!hash.contains("libhook"));
}

#[test]
fn a_new_kernel_release_and_a_new_libc_are_each_seen() {
    let root = fake_host("each");
    let before = identity(&root);
    put(&root, "proc/sys/kernel/osrelease", "6.9.0\n");
    put(&root, "lib/x86_64-linux-gnu/libc.so.6", "libc-two");
    let after = identity(&root);
    assert_ne!(
        before.get("host.kernel_release"),
        after.get("host.kernel_release")
    );
    assert_ne!(
        before.get("host.loader.libc.sha256"),
        after.get("host.loader.libc.sha256")
    );
}

#[test]
fn a_different_hermit_binary_or_a_missing_version_is_seen() {
    let root = fake_host("hermitid");
    let other = HermitIdentity {
        sha256: Some("2".repeat(64)),
        version: None,
    };
    let after = run(&root, &other, &Inputs::default());
    assert_eq!(
        after.get("hermit.path.sha256"),
        Some("2".repeat(64).as_str())
    );
    assert_eq!(after.get("hermit.version"), None);
}

#[test]
fn aarch64_and_musl_loaders_are_found() {
    for (name, libc, ld) in [
        (
            "aarch64",
            "lib/aarch64-linux-gnu/libc.so.6",
            "lib/ld-linux-aarch64.so.1",
        ),
        (
            "musl",
            "lib/libc.musl-x86_64.so.1",
            "lib/ld-musl-x86_64.so.1",
        ),
    ] {
        let root = scratch(name);
        put(&root, libc, "libc");
        put(&root, ld, "ld");
        let id = identity(&root);
        assert!(id.get("host.loader.libc.sha256").is_some(), "{name}");
        assert!(id.get("host.loader.ld_so.sha256").is_some(), "{name}");
    }
}

#[test]
fn a_container_marker_is_named() {
    let root = fake_host("container");
    put(&root, ".dockerenv", "");
    assert_eq!(identity(&root).get("host.container"), Some("docker"));
}

#[test]
fn a_source_the_host_lacks_is_omitted_not_invented() {
    let root = scratch("bare");
    let none = HermitIdentity {
        sha256: None,
        version: None,
    };
    let inputs = Inputs {
        command: vec!["definitely-not-installed".to_string()],
        env: BTreeMap::new(),
        ..Inputs::default()
    };
    let id = run(&root, &none, &inputs);
    for name in [
        "host.kernel_release",
        "host.cpu_model",
        "host.loader.libc.sha256",
        "hermit.path.sha256",
        "target.exe.sha256",
    ] {
        assert_eq!(id.get(name), None, "{name}");
    }
}

#[test]
fn a_target_that_is_not_a_regular_file_is_not_hashed() {
    let root = fake_host("device");
    for program in ["/dev/null", "/"] {
        let inputs = Inputs {
            command: vec![program.to_string()],
            ..Inputs::default()
        };
        assert_eq!(
            run(&root, &hermit(), &inputs).get("target.exe.sha256"),
            None,
            "{program}"
        );
    }
}

#[test]
fn guest_variable_values_are_hashed_only_for_allowlisted_names() {
    let root = fake_host("guestenv");
    let with = |pairs: &[(&str, &str)]| {
        let env: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        run(
            &root,
            &hermit(),
            &Inputs {
                env,
                ..Inputs::default()
            },
        )
    };
    let base = with(&[("PATH", "/bin"), ("API_TOKEN", "hunter2")]);
    // A secret's value changes nothing that is recorded, so its hash cannot be guessed offline.
    let other_secret = with(&[("PATH", "/bin"), ("API_TOKEN", "hunter3")]);
    assert_eq!(base, other_secret);
    assert_eq!(base.get("env.guest.names"), Some("API_TOKEN,PATH"));
    for (_, value) in base.fields() {
        assert!(!value.contains("hunter2"));
    }
    // An allowlisted value does matter, and so does a new name.
    let other_path = with(&[("PATH", "/usr/bin"), ("API_TOKEN", "hunter2")]);
    assert_ne!(
        base.get("env.guest.allowlisted.sha256"),
        other_path.get("env.guest.allowlisted.sha256")
    );
    let more = with(&[("PATH", "/bin"), ("API_TOKEN", "x"), ("NEW_VAR", "y")]);
    assert_ne!(base.get("env.guest.names"), more.get("env.guest.names"));
    let locale = with(&[("PATH", "/bin"), ("LC_TIME", "C")]);
    assert_ne!(
        locale.get("env.guest.allowlisted.sha256"),
        with(&[("PATH", "/bin")]).get("env.guest.allowlisted.sha256")
    );
}

#[test]
fn extra_hermit_environment_names_are_part_of_the_guest_identity() {
    let root = fake_host("extra");
    let inputs = Inputs {
        extra: vec!["ZED".to_string(), "ALPHA".to_string()],
        ..Inputs::default()
    };
    let id = run(&root, &hermit(), &inputs);
    assert_eq!(id.get("env.guest.names"), Some("ALPHA,PATH,ZED"));
}

#[test]
fn a_very_long_name_list_falls_back_to_its_hash() {
    let root = fake_host("longnames");
    let env: BTreeMap<String, String> = (0..100)
        .map(|i| (format!("SOME_LONG_VARIABLE_NAME_{i}"), String::new()))
        .collect();
    let id = run(
        &root,
        &hermit(),
        &Inputs {
            env,
            ..Inputs::default()
        },
    );
    assert_eq!(id.get("env.guest.names"), None);
    assert_eq!(id.get("env.guest.names.sha256").unwrap().len(), 64);
}

#[test]
fn the_base_env_mode_comes_from_the_hermit_arguments() {
    let root = fake_host("baseenv");
    let mode = |args: &[&str]| {
        let inputs = Inputs {
            args: args.iter().map(|a| a.to_string()).collect(),
            ..Inputs::default()
        };
        run(&root, &hermit(), &inputs)
            .get("env.base_env")
            .map(str::to_string)
    };
    assert_eq!(mode(&[]).as_deref(), Some("default"));
    assert_eq!(mode(&["--base-env=minimal"]).as_deref(), Some("minimal"));
    assert_eq!(mode(&["--base-env=host"]).as_deref(), Some("host"));
    assert_eq!(mode(&["--base-env", "empty"]).as_deref(), Some("empty"));
    assert_eq!(mode(&["--base-env=surprise"]).as_deref(), Some("unknown"));
    assert_eq!(mode(&["--base-env"]).as_deref(), Some("unknown"));
}

#[test]
fn no_value_carries_a_path_a_user_name_or_a_host_name() {
    let root = fake_host("private");
    let id = identity(&root);
    let root_text = root.display().to_string();
    for (name, value) in id.fields() {
        assert!(!value.contains(&root_text), "{name}: {value}");
        assert!(!value.contains("/Users/"), "{name}: {value}");
        assert!(!value.contains("/home/"), "{name}: {value}");
    }
}
