//! Checks of the settings before the first spawn: every path is one the VM program's option parser cannot misread, the two
//! VM programs are regular files (not links to something else), and each program and the library directory match the
//! SHA-256 pins the settings give. The allow-list matches programs by their path text, so what sits at a path is checked
//! here. The program directory may be writable by the same user, so this closes accidents, not a hostile local user.
use crate::error::{InfraError, InfraKind};
use crate::host::HostConfig;
use hairspring_core::environment::sha256_file;
use hairspring_core::point::encoding::sha256_hex;
use std::path::{Component, Path};

fn path_ok(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false;
    };
    path.is_absolute()
        && !text.contains([',', '='])
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
        && !path.components().any(|c| matches!(c, Component::ParentDir))
}

fn settings_problem(host: &HostConfig) -> Option<&'static str> {
    let named: [(&str, &Path); 6] = [
        ("the bhyve path", &host.bhyve),
        ("the bhyvectl path", &host.bhyvectl),
        ("the library directory", &host.lib_dir),
        ("the state directory", &host.state_dir),
        ("the kernel path", &host.kernel),
        ("the initial RAM disk path", &host.initrd),
    ];
    named
        .iter()
        .find(|(_, p)| !path_ok(p))
        .map(|(what, _)| match *what {
            "the bhyve path" => "the bhyve path is not an absolute path of plain characters",
            "the bhyvectl path" => "the bhyvectl path is not an absolute path of plain characters",
            "the library directory" => {
                "the library directory is not an absolute path of plain characters"
            }
            "the state directory" => {
                "the state directory is not an absolute path of plain characters"
            }
            "the kernel path" => "the kernel path is not an absolute path of plain characters",
            _ => "the initial RAM disk path is not an absolute path of plain characters",
        })
}

fn regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// The manifest of the files directly in `dir`: one `<sha256>  <name>` line each, sorted by name. Anything that is not a
/// regular file (a link, which the loader would follow, or a directory) is refused, not skipped.
pub fn lib_manifest(dir: &Path) -> std::io::Result<String> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a regular file",
            ));
        }
        names.push(
            entry.file_name().into_string().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "not plain text")
            })?,
        );
    }
    names.sort();
    let mut text = String::new();
    for name in names {
        text.push_str(&format!("{}  {name}\n", sha256_file(&dir.join(&name))?));
    }
    Ok(text)
}

/// SHA-256 of the manifest text, which is what a library pin holds.
pub fn lib_manifest_pin(dir: &Path) -> std::io::Result<String> {
    Ok(sha256_hex(lib_manifest(dir)?.as_bytes()))
}

fn mismatch(what: &str) -> InfraError {
    InfraError::new(InfraKind::PinMismatch, what)
}

/// Runs every check. Nothing is spawned before this passes.
pub fn verify(host: &HostConfig) -> Result<(), InfraError> {
    if let Some(problem) = settings_problem(host) {
        return Err(InfraError::new(InfraKind::Settings, problem));
    }
    if host.require_regular_files {
        for (what, path) in [("bhyve", &host.bhyve), ("bhyvectl", &host.bhyvectl)] {
            if !regular(path) {
                return Err(mismatch(&format!("{what} is not a regular file")));
            }
        }
    }
    let pinned = [
        ("bhyve", &host.pins.bhyve, &host.bhyve),
        ("bhyvectl", &host.pins.bhyvectl, &host.bhyvectl),
    ];
    for (what, pin, path) in pinned {
        let Some(pin) = pin else { continue };
        let found = sha256_file(path).map_err(|e| InfraError::io("read a pinned program", &e))?;
        if found != *pin {
            return Err(mismatch(&format!("{what} differs from its pin")));
        }
    }
    if let Some(pin) = &host.pins.lib_manifest {
        let found = lib_manifest_pin(&host.lib_dir).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                mismatch("the library directory holds something that is not a regular file")
            } else {
                InfraError::io("read the library directory", &e)
            }
        })?;
        if found != *pin {
            return Err(mismatch("the library directory differs from its pin"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-dhyve-pins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(d.join("bhyve"), b"one").unwrap();
        std::fs::write(d.join("bhyvectl"), b"two").unwrap();
        std::fs::write(d.join("lib/liba.so"), b"a").unwrap();
        std::fs::write(d.join("lib/libb.so"), b"b").unwrap();
        d
    }

    fn host(d: &Path) -> HostConfig {
        HostConfig::new(
            d.join("bhyve"),
            d.join("bhyvectl"),
            d.join("lib"),
            d.join("st"),
            d.join("kernel"),
            d.join("initrd"),
        )
    }

    #[test]
    fn plain_settings_with_no_pins_pass() {
        let d = scratch("plain");
        assert_eq!(verify(&host(&d)), Ok(()));
    }

    #[test]
    fn a_symbolic_link_for_a_vm_program_is_refused_unless_links_are_allowed() {
        let d = scratch("link");
        std::fs::remove_file(d.join("bhyve")).unwrap();
        std::os::unix::fs::symlink("/bin/sh", d.join("bhyve")).unwrap();
        let err = verify(&host(&d)).unwrap_err();
        assert_eq!(err.kind, InfraKind::PinMismatch);
        let mut lenient = host(&d);
        lenient.require_regular_files = false;
        assert_eq!(verify(&lenient), Ok(()));
    }

    #[test]
    fn a_program_whose_hash_differs_from_its_pin_is_refused() {
        let d = scratch("pin");
        let mut h = host(&d);
        h.pins.bhyve = Some(sha256_file(&d.join("bhyve")).unwrap());
        h.pins.bhyvectl = Some(sha256_file(&d.join("bhyvectl")).unwrap());
        assert_eq!(verify(&h), Ok(()));
        std::fs::write(d.join("bhyvectl"), b"changed").unwrap();
        let err = verify(&h).unwrap_err();
        assert_eq!(err.kind, InfraKind::PinMismatch);
        assert!(err.detail.contains("bhyvectl"), "{err}");
    }

    #[test]
    fn a_library_directory_whose_manifest_differs_from_its_pin_is_refused() {
        let d = scratch("manifest");
        let mut h = host(&d);
        h.pins.lib_manifest = Some(lib_manifest_pin(&d.join("lib")).unwrap());
        assert_eq!(verify(&h), Ok(()));
        std::fs::write(d.join("lib/libb.so"), b"tampered").unwrap();
        assert_eq!(verify(&h).unwrap_err().kind, InfraKind::PinMismatch);
        std::fs::write(d.join("lib/libb.so"), b"b").unwrap();
        assert_eq!(verify(&h), Ok(()));
        std::fs::write(d.join("lib/libc.so"), b"extra").unwrap();
        assert_eq!(verify(&h).unwrap_err().kind, InfraKind::PinMismatch);
    }

    #[test]
    fn a_library_directory_holding_a_link_or_a_directory_is_refused() {
        let d = scratch("libs-links");
        let mut h = host(&d);
        h.pins.lib_manifest = Some(lib_manifest_pin(&d.join("lib")).unwrap());
        std::os::unix::fs::symlink("/bin/sh", d.join("lib/libc.so")).unwrap();
        let err = verify(&h).unwrap_err();
        assert_eq!(err.kind, InfraKind::PinMismatch);
        assert!(err.detail.contains("not a regular file"), "{err}");
        assert!(lib_manifest(&d.join("lib")).is_err());
        std::fs::remove_file(d.join("lib/libc.so")).unwrap();
        std::fs::create_dir(d.join("lib/sub")).unwrap();
        assert_eq!(verify(&h).unwrap_err().kind, InfraKind::PinMismatch);
    }

    #[test]
    fn the_manifest_is_one_sorted_line_per_file() {
        let d = scratch("lines");
        let text = lib_manifest(&d.join("lib")).unwrap();
        let names: Vec<&str> = text
            .lines()
            .map(|l| l.split("  ").nth(1).unwrap())
            .collect();
        assert_eq!(names, ["liba.so", "libb.so"]);
    }

    #[test]
    fn a_settings_path_the_option_parser_could_misread_is_refused() {
        let d = scratch("paths");
        for bad in [
            "/w/a,b",
            "/w/a=b",
            "/w/a b",
            "/w/a\u{1b}b",
            "/w/../x",
            "relative/path",
        ] {
            for field in 0..6 {
                let mut h = host(&d);
                let slot = [
                    &mut h.bhyve,
                    &mut h.bhyvectl,
                    &mut h.lib_dir,
                    &mut h.state_dir,
                    &mut h.kernel,
                    &mut h.initrd,
                ]
                .into_iter()
                .nth(field)
                .unwrap();
                *slot = PathBuf::from(bad);
                let err = verify(&h).unwrap_err();
                assert_eq!(err.kind, InfraKind::Settings, "{bad} in field {field}");
                assert!(!err.to_string().contains(bad), "the text is not echoed");
            }
        }
    }
}
