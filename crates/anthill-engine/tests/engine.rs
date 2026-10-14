use anthill_core::bundle::Bundle;
use anthill_core::demo::LockLease;
use anthill_core::explore::{branching, Config as ExploreConfig};
use anthill_core::replay::replay;
use anthill_core::Sim;
use anthill_engine::config::{Budget, Config, ConfigError, TargetKind};
use anthill_engine::demos::default_registry;
use anthill_engine::engine::{Engine, RunRequest};
use anthill_engine::erased::{Erased, Registry};
use anthill_engine::format::{to_junit, to_sarif, to_text};
use anthill_engine::report::RunStatus;
use anthill_engine::EngineError;
use std::sync::Arc;

fn engine() -> Engine {
    let registry = Arc::new(default_registry());
    let config = Config::from_registry(&registry);
    Engine::new(registry, config)
}

// ---- registry / erased ----
#[test]
fn registry_lists_and_instantiates_registered_targets() {
    let r = default_registry();
    let names = r.names();
    assert!(names.contains(&"lock-lease".to_string()));
    assert!(names.contains(&"pubsub-buggy".to_string()));
    assert!(r.instantiate("lock-lease").is_some());
    assert!(r.instantiate("nope").is_none());
    assert!(r.description("lock-lease").unwrap().len() > 5);
}

#[test]
fn erased_sim_behaves_like_the_original_sim() {
    let erased = Registry::new_with("lock-lease", "d", LockLease::new)
        .instantiate("lock-lease")
        .unwrap();
    let f = branching(
        &erased,
        &ExploreConfig {
            seed: 3,
            ..ExploreConfig::default()
        },
    )
    .expect("bug found via erased sim");
    assert!(replay(&erased, &f.path).unwrap().state.violated());
    // identical result to the un-erased sim for the same seed
    let direct = branching(
        &LockLease::new(),
        &ExploreConfig {
            seed: 3,
            ..ExploreConfig::default()
        },
    )
    .unwrap();
    assert_eq!(f.path, direct.path);
    assert!(Erased::state_debug(&erased).contains("LockLease"));
}

// ---- config ----
#[test]
fn config_parses_all_target_kinds_with_defaults() {
    let c = Config::parse(
        r#"
[target.a]
kind = "sim"
sim = "lock-lease"
[target.b]
kind = "process"
command = ["./svc", "--flag"]
env = { MODE = "test" }
[target.b.budget]
max_trials = 50
[target.c]
kind = "wasm"
module = "m.wat"
"#,
    )
    .unwrap();
    assert_eq!(c.targets.len(), 3);
    assert!(matches!(&c.targets["a"].kind, TargetKind::Sim { sim } if sim == "lock-lease"));
    assert!(
        matches!(&c.targets["b"].kind, TargetKind::Process { command, env, .. } if command.len() == 2 && env["MODE"] == "test")
    );
    assert_eq!(c.targets["b"].budget.max_trials, 50);
    assert_eq!(c.targets["a"].budget, Budget::default());
}

#[test]
fn config_rejects_invalid_definitions() {
    assert!(matches!(
        Config::parse("[target.x]\nkind = \"sim\"\n"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        Config::parse("[target.x]\nkind = \"process\"\ncommand = []\n"),
        Err(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        Config::parse("[target.x]\nkind = \"nope\"\n"),
        Err(ConfigError::Parse(_))
    ));
    assert!(matches!(
        Config::parse("not toml ["),
        Err(ConfigError::Parse(_))
    ));
    assert!(matches!(
        Config::parse("[target.\"bad name!\"]\nkind=\"sim\"\nsim=\"x\"\n"),
        Err(ConfigError::Invalid(_))
    ));
}

#[test]
fn budget_clamp_caps_runaway_values() {
    let b = Budget {
        max_steps: u64::MAX,
        max_trials: u64::MAX,
        wall_secs: u64::MAX,
    }
    .clamp();
    assert!(b.max_steps <= 50_000_000 && b.wall_secs <= 3600 && b.max_trials <= 100_000);
    assert_eq!(Budget::default().clamp(), Budget::default());
}

// ---- engine / sim backend ----
#[test]
fn engine_finds_shrinks_and_reports_the_lock_bug() {
    let out = engine()
        .run(&RunRequest::new("lock-lease", 7), &mut |_| {})
        .unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    let f = out.failure.expect("failure report");
    assert!(f.shrunk_len <= f.original_len && f.shrunk_len >= 1);
    assert_eq!(f.steps.len(), f.shrunk_len);
    assert!(f
        .steps
        .iter()
        .all(|s| !s.label.is_empty() && !s.state.is_empty()));
    assert!(f.steps.last().unwrap().state.contains("violated: true"));
    let b = Bundle::decode(&f.bundle).unwrap();
    assert_eq!(
        (b.target.as_str(), b.backend.as_str()),
        ("lock-lease", "sim")
    );
    assert_eq!(b.path.len(), f.shrunk_len);
}

#[test]
fn engine_replay_reproduces_a_bundle() {
    let e = engine();
    let f = e
        .run(&RunRequest::new("pubsub-buggy", 5), &mut |_| {})
        .unwrap()
        .failure
        .unwrap();
    let r = e.replay(&Bundle::decode(&f.bundle).unwrap()).unwrap();
    assert!(r.reproduced);
    assert_eq!(r.steps, f.steps);
}

#[test]
fn engine_passes_the_fixed_target_within_budget() {
    let req = RunRequest::new("pubsub-atomic", 1).with_budget(Budget {
        max_steps: 50_000,
        max_trials: 10,
        wall_secs: 30,
    });
    let out = engine().run(&req, &mut |_| {}).unwrap();
    assert_eq!(out.status, RunStatus::Passed);
    assert!(out.failure.is_none() && out.steps_executed > 0);
}

#[test]
fn engine_runs_are_deterministic_for_a_seed() {
    let a = engine()
        .run(&RunRequest::new("lock-lease", 11), &mut |_| {})
        .unwrap()
        .failure
        .unwrap()
        .bundle;
    let b = engine()
        .run(&RunRequest::new("lock-lease", 11), &mut |_| {})
        .unwrap()
        .failure
        .unwrap()
        .bundle;
    assert_eq!(a, b);
}

#[test]
fn engine_emits_progress_events() {
    let mut events = 0;
    engine()
        .run(
            &RunRequest::new("pubsub-atomic", 2).with_budget(Budget {
                max_steps: 30_000,
                max_trials: 1,
                wall_secs: 30,
            }),
            &mut |_| events += 1,
        )
        .unwrap();
    assert!(events >= 1);
}

#[test]
fn engine_rejects_unknown_targets_and_missing_backends() {
    let e = engine();
    assert!(matches!(
        e.run(&RunRequest::new("nope", 1), &mut |_| {}),
        Err(EngineError::UnknownTarget(_))
    ));
    let registry = Arc::new(default_registry());
    let cfg = Config::parse("[target.p]\nkind=\"process\"\ncommand=[\"/bin/true\"]\n").unwrap();
    let e2 = Engine::new(registry, cfg);
    assert!(matches!(
        e2.run(&RunRequest::new("p", 1), &mut |_| {}),
        Err(EngineError::BackendUnavailable(_))
    ));
}

#[test]
fn engine_clamps_oversized_request_budgets() {
    let req = RunRequest::new("pubsub-atomic", 1).with_budget(Budget {
        max_steps: 1_000,
        max_trials: 1,
        wall_secs: 1,
    });
    // must complete quickly and not error even though budget is tiny
    assert_eq!(
        engine().run(&req, &mut |_| {}).unwrap().status,
        RunStatus::Passed
    );
}

// ---- formats ----
fn failed() -> (String, anthill_engine::report::RunOutcome) {
    (
        "lock-lease".to_string(),
        engine()
            .run(&RunRequest::new("lock-lease", 7), &mut |_| {})
            .unwrap(),
    )
}

#[test]
fn text_report_lists_steps_and_bundle() {
    let (t, o) = failed();
    let s = to_text(&t, &o);
    assert!(s.contains("FAILED") && s.contains("lock-lease") && s.contains("anthill-bundle v2"));
}

#[test]
fn junit_reports_failures_and_escapes_xml() {
    let (t, o) = failed();
    let xml = to_junit(&[(t, o)]);
    assert!(xml.starts_with("<?xml") && xml.contains("<testsuite") && xml.contains("<failure"));
    assert!(!xml.contains("<<") && xml.matches("<testcase").count() == 1);
    let evil = to_junit(&[(
        "a<b>&\"".to_string(),
        anthill_engine::report::RunOutcome::error("x<y>"),
    )]);
    assert!(evil.contains("a&lt;b&gt;&amp;&quot;") && evil.contains("x&lt;y&gt;"));
}

#[test]
fn sarif_is_valid_json_with_results() {
    let (t, o) = failed();
    let v: serde_json::Value = serde_json::from_str(&to_sarif(&[(t, o)])).unwrap();
    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 1);
    assert_eq!(v["runs"][0]["results"][0]["level"], "error");
}

// ---- doctor ----
#[test]
fn doctor_passes_for_deterministic_targets() {
    let r = engine().doctor("lock-lease", 5, 60).unwrap();
    assert!(r.ok && r.seeds_checked == 5 && r.divergence.is_none());
}

#[derive(Clone, Debug)]
struct Leaky(u32);
impl anthill_core::Sim for Leaky {
    type Action = u64;
    fn enabled(&self) -> Vec<u64> {
        if self.0 > 4 {
            return vec![];
        }
        // BUG under test: iteration order of a freshly built HashMap differs per call.
        let m: std::collections::HashMap<u64, ()> = (0..16).map(|i| (i, ())).collect();
        m.keys().copied().collect()
    }
    fn action_id(a: &u64) -> anthill_core::ActionId {
        *a
    }
    fn apply(&self, _a: u64) -> Self {
        Leaky(self.0 + 1)
    }
    fn signature(&self) -> u64 {
        self.0 as u64
    }
    fn violated(&self) -> bool {
        false
    }
}

#[test]
fn doctor_reports_the_first_divergence_for_a_leaky_target() {
    let registry = Arc::new(Registry::new_with(
        "leaky",
        "has a HashMap-order leak",
        || Leaky(0),
    ));
    let e = Engine::new(registry.clone(), Config::from_registry(&registry));
    let r = e.doctor("leaky", 3, 10).unwrap();
    assert!(!r.ok);
    let d = r.divergence.expect("divergence reported");
    assert_eq!(d.seed, 1);
}

#[test]
fn doctor_rejects_unknown_targets_and_unsupported_backends() {
    assert!(matches!(
        engine().doctor("nope", 1, 1),
        Err(EngineError::UnknownTarget(_))
    ));
    let registry = Arc::new(default_registry());
    let cfg = Config::parse("[target.p]\nkind=\"process\"\ncommand=[\"/bin/true\"]\n").unwrap();
    assert!(matches!(
        Engine::new(registry, cfg).doctor("p", 1, 1),
        Err(EngineError::BackendUnavailable(_))
    ));
}
