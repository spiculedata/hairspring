//! The default `trial_dir` is `/tmp`, where Hermit made its temporary files (the engine clears Hermit's environment,
//! so it never used the engine's own `TMPDIR`). This runs alone in its process because it sets an environment variable.
use hairspring_hermit::HermitSettings;
use std::path::Path;

#[test]
fn the_default_ignores_the_engines_own_tmpdir() {
    std::env::set_var("TMPDIR", "/somewhere/else");
    assert_eq!(HermitSettings::default().trial_dir, Path::new("/tmp"));
}
