//! `parallelism` on process targets: default, range and where it is allowed.
use hairspring_engine::config::{Config, ConfigError, TargetKind};

fn parallelism(extra: &str) -> Result<u32, ConfigError> {
    let c = Config::parse(&format!(
        "[target.x]\nkind = \"process\"\ncommand = [\"./svc\"]\n{extra}\n"
    ))?;
    match &c.targets["x"].kind {
        TargetKind::Process { parallelism, .. } => Ok(*parallelism),
        other => panic!("expected a process target, got {}", other.name()),
    }
}

#[test]
fn trials_run_one_at_a_time_unless_asked() {
    assert_eq!(parallelism("").unwrap(), 1);
    assert_eq!(parallelism("parallelism = 4").unwrap(), 4);
}

#[test]
fn parallelism_is_range_checked() {
    assert!(matches!(
        parallelism("parallelism = 0"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        parallelism("parallelism = 65"),
        Err(ConfigError::Invalid(_))
    ));
    assert_eq!(parallelism("parallelism = 64").unwrap(), 64);
}

#[test]
fn parallelism_is_rejected_on_other_target_kinds() {
    let err = Config::parse("[target.s]\nkind = \"sim\"\nsim = \"lock-lease\"\nparallelism = 2\n")
        .unwrap_err();
    assert!(matches!(&err, ConfigError::Invalid(m) if m.contains("process")));
}
