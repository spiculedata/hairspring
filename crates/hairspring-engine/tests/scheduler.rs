//! Scheduler options on `process` targets: parsing, defaults and validation.
use hairspring_engine::config::{Config, ConfigError, TargetKind};
use hairspring_engine::scheduler::{Heuristic, SchedulerSpec};

fn parse(extra: &str) -> Result<Config, ConfigError> {
    Config::parse(&format!(
        "[target.x]\nkind = \"process\"\ncommand = [\"./svc\"]\n{extra}\n"
    ))
}

fn scheduler_of(c: &Config) -> SchedulerSpec {
    match &c.targets["x"].kind {
        TargetKind::Process { scheduler, .. } => scheduler.clone(),
        other => panic!("expected a process target, got {}", other.name()),
    }
}

#[test]
fn default_scheduler_is_seeded_random() {
    let s = scheduler_of(&parse("").unwrap());
    assert_eq!(s, SchedulerSpec::default());
    assert_eq!(s.effective_heuristic(), Heuristic::Random);
    assert!(!s.chaos);
}

#[test]
fn chaos_without_an_explicit_heuristic_uses_no_heuristic() {
    let s = scheduler_of(&parse("chaos = true").unwrap());
    assert!(s.chaos);
    assert_eq!(s.effective_heuristic(), Heuristic::None);
}

#[test]
fn every_scheduler_option_parses() {
    let s = scheduler_of(
        &parse(
            "heuristic = \"sticky-random\"\nsticky_permille = 900\nmax_timeslice_ns = 1000000\nskid_margin = 25",
        )
        .unwrap(),
    );
    assert_eq!(s.effective_heuristic(), Heuristic::StickyRandom);
    assert_eq!(s.sticky_permille, Some(900));
    assert_eq!(s.max_timeslice_ns, Some(1_000_000));
    assert_eq!(s.skid_margin, Some(25));
}

#[test]
fn chaos_with_the_random_heuristic_is_rejected() {
    // Random picks uniformly and ignores the priorities chaos mode sets.
    let err = parse("chaos = true\nheuristic = \"random\"").unwrap_err();
    assert!(matches!(&err, ConfigError::Invalid(m) if m.contains("chaos") && m.contains("random")));
}

#[test]
fn stickiness_needs_the_sticky_random_heuristic_and_a_valid_range() {
    assert!(matches!(
        parse("sticky_permille = 500"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        parse("heuristic = \"sticky-random\"\nsticky_permille = 1001"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(parse("heuristic = \"sticky-random\"\nsticky_permille = 1000").is_ok());
}

#[test]
fn skid_margin_and_timeslice_are_range_checked() {
    assert!(matches!(
        parse("skid_margin = 0"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        parse("skid_margin = 10001"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(parse("skid_margin = 10000").is_ok());
    assert!(matches!(
        parse("max_timeslice_ns = 5"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(parse("max_timeslice_ns = 10").is_ok());
}

#[test]
fn unknown_heuristic_names_are_a_parse_error() {
    assert!(matches!(
        parse("heuristic = \"fastest\""),
        Err(ConfigError::Parse(_))
    ));
}

#[test]
fn scheduler_options_on_other_target_kinds_are_rejected() {
    let err = Config::parse("[target.s]\nkind = \"sim\"\nsim = \"lock-lease\"\nchaos = true\n")
        .unwrap_err();
    assert!(matches!(&err, ConfigError::Invalid(m) if m.contains("process")));
}
