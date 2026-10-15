//! `hang_is_violation` on process targets: default, parsing and where it is allowed.
use hairspring_engine::config::{Config, ConfigError, TargetKind};

fn hang_flag(extra: &str) -> Result<bool, ConfigError> {
    let c = Config::parse(&format!(
        "[target.x]\nkind = \"process\"\ncommand = [\"./svc\"]\n{extra}\n"
    ))?;
    match &c.targets["x"].kind {
        TargetKind::Process {
            hang_is_violation, ..
        } => Ok(*hang_is_violation),
        other => panic!("expected a process target, got {}", other.name()),
    }
}

#[test]
fn a_hang_is_inconclusive_unless_the_target_says_otherwise() {
    assert!(!hang_flag("").unwrap());
    assert!(hang_flag("hang_is_violation = true").unwrap());
    assert!(!hang_flag("hang_is_violation = false").unwrap());
}

#[test]
fn hang_is_violation_is_rejected_on_other_target_kinds() {
    let err = Config::parse(
        "[target.s]\nkind = \"sim\"\nsim = \"lock-lease\"\nhang_is_violation = true\n",
    )
    .unwrap_err();
    assert!(matches!(&err, ConfigError::Invalid(m) if m.contains("process")));
}
