//! What the backend records about the host, read from a fake root so the tests do not depend on the machine.
mod common;
use common::scratch;
use hairspring_hermit::host_identity::{collect, GuestInputs, HostSources};
use std::collections::BTreeMap;
use std::path::Path;

const CPUINFO: &str = "processor\t: 0\nmodel name\t: Fake CPU @ 3.0GHz\nflags\t\t: sse2 avx2 rdrand\n\nprocessor\t: 1\nmodel name\t: Fake CPU @ 3.0GHz\nflags\t\t: sse2 avx2 rdrand\n";

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn fake_host(name: &str) -> std::path::PathBuf {
    let root = scratch(name);
    put(&root, "proc/sys/kernel/osrelease", "6.8.0-45-generic\n");
    put(&root, "proc/cpuinfo", CPUINFO);
    put(&root, "etc/ld.so.cache", "cache-one");
    put(&root, "lib/x86_64-linux-gnu/libc.so.6", "libc-one");
    put(&root, "lib64/ld-linux-x86-64.so.2", "ld-one");
    root
}

fn guest<'a>(command: &'a [String], env: &'a BTreeMap<String, String>) -> GuestInputs<'a> {
    GuestInputs {
        command,
        cwd: None,
        env,
    }
}

fn identity(root: &Path) -> hairspring_core::environment::EnvironmentIdentity {
    let hermit = root.join("hermit-bin");
    put(root, "hermit-bin", "hermit-one");
    let command = vec!["/bin/sh".to_string()];
    let env = BTreeMap::from([("PATH".to_string(), "/bin".to_string())]);
    collect(
        &HostSources::rooted(root),
        &hermit,
        Some("hermit 0.1.0"),
        &guest(&command, &env),
    )
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
        "hermit.path.sha256",
        "env.guest.sha256",
        "target.exe.sha256",
    ] {
        let value = id.get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(value.len(), 64, "{name}: {value}");
    }
    assert_eq!(id.get("hermit.version"), Some("hermit 0.1.0"));
    assert_eq!(id.get("env.base_env"), Some("minimal"));
    assert_eq!(id.get("host.container"), Some("none"));
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
fn a_new_kernel_release_a_new_hermit_and_a_new_libc_are_each_seen() {
    let root = fake_host("each");
    let before = identity(&root);
    put(&root, "proc/sys/kernel/osrelease", "6.9.0\n");
    put(&root, "lib/x86_64-linux-gnu/libc.so.6", "libc-two");
    let after_host = identity(&root);
    assert_ne!(
        before.get("host.kernel_release"),
        after_host.get("host.kernel_release")
    );
    assert_ne!(
        before.get("host.loader.libc.sha256"),
        after_host.get("host.loader.libc.sha256")
    );
    put(&root, "hermit-bin", "hermit-two");
    let hermit = root.join("hermit-bin");
    let command = vec!["/bin/sh".to_string()];
    let env = BTreeMap::new();
    let after_hermit = collect(
        &HostSources::rooted(&root),
        &hermit,
        None,
        &guest(&command, &env),
    );
    assert_ne!(
        after_host.get("hermit.path.sha256"),
        after_hermit.get("hermit.path.sha256")
    );
    assert_eq!(after_hermit.get("hermit.version"), None);
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
    let hermit = root.join("missing-hermit");
    let command = vec!["definitely-not-installed".to_string()];
    let env = BTreeMap::new();
    let id = collect(
        &HostSources::rooted(&root),
        &hermit,
        None,
        &guest(&command, &env),
    );
    for name in [
        "host.kernel_release",
        "host.cpu_model",
        "host.loader.libc.sha256",
        "hermit.path.sha256",
        "target.exe.sha256",
    ] {
        assert_eq!(id.get(name), None, "{name}");
    }
    assert_eq!(id.get("env.base_env"), Some("minimal"));
}

#[test]
fn guest_environment_keys_and_values_change_its_hash() {
    let root = fake_host("guestenv");
    let hermit = root.join("hermit-bin");
    put(&root, "hermit-bin", "h");
    let command = vec!["/bin/sh".to_string()];
    let hash = |pairs: &[(&str, &str)]| {
        let env: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        collect(
            &HostSources::rooted(&root),
            &hermit,
            None,
            &guest(&command, &env),
        )
        .get("env.guest.sha256")
        .map(str::to_string)
    };
    assert_ne!(hash(&[("A", "1")]), hash(&[("A", "2")]));
    assert_ne!(hash(&[("A", "1")]), hash(&[("A", "1"), ("B", "1")]));
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
