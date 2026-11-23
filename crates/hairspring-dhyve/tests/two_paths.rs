//! The session driver and `Backend::run` reach the host through the same functions. Against the fake, one control restore
//! through the driver's calls and one through `DhyveBackend::run` (one trial, the same seed) start the VM with the same
//! arguments, use the same VM-name form and the same number of destroys for the start, and judge it the same way. A second
//! test fails if either stops calling the prepare, the restore or the judge.
mod common;
use common::{quick, tiny_bounds, Rig};
use hairspring_dhyve::fwcfg::Inputs;
use hairspring_dhyve::judge::judge_trial;
use hairspring_dhyve::vmname::VmName;
use hairspring_dhyve::{DhyveBackend, BACKEND_ID};
use hairspring_engine::backend::{Backend, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::RunStatus;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::verdict::Verdict;
use std::collections::BTreeMap;

const SEED: u64 = 1007;

fn spec() -> TargetSpec {
    TargetSpec {
        name: "demo".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(5),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
            pass_env: false,
            backend: Some(BACKEND_ID.into()),
        },
        budget: Budget::default(),
    }
}

fn destroys(rig: &Rig) -> usize {
    rig.calls()
        .iter()
        .filter(|c| c.contains("--destroy"))
        .count()
}

#[test]
fn a_restore_through_the_drivers_calls_and_one_through_run_start_the_vm_alike() {
    let by_driver = Rig::new("p-driver");
    let session = by_driver.session();
    let prepared = session.prepare(Inputs { seed: SEED, job: 0 }).unwrap();
    let trial = session
        .restore_trial(&prepared, 0, Inputs { seed: SEED, job: 0 })
        .unwrap();
    let judged = judge_trial(&trial, 5, false).unwrap();

    let by_run = Rig::new("p-run");
    let backend = DhyveBackend::with_settings(by_run.host.clone(), quick(), tiny_bounds());
    let budget = Budget {
        max_trials: 1,
        ..Budget::default()
    };
    let outcome = backend
        .run(&spec(), &RunRequest::new("demo", SEED), budget, &mut |_| {
            true
        })
        .unwrap();

    assert_eq!(judged.verdict, Verdict::Pass);
    assert_eq!(outcome.status, RunStatus::Passed, "{:?}", outcome.error);
    let (a, b) = (by_driver.start_args(), by_run.start_args());
    assert_eq!(a.len(), 2, "a prepare and a restore");
    assert_eq!(a, b, "the VM program was given the same arguments");
    let names: Vec<VmName> = a
        .iter()
        .map(|l| {
            VmName::parse(l.split(' ').next_back().unwrap()).expect("a name of the session form")
        })
        .collect();
    assert_eq!(names[1].run_number(), 2);
    assert_eq!(
        destroys(&by_driver),
        destroys(&by_run),
        "the same destroys around the starts"
    );
    assert_eq!(destroys(&by_driver), 4);
    assert_eq!(
        by_driver.trials(),
        by_run.trials(),
        "the guest read the same seed"
    );
}

#[test]
fn the_driver_and_run_both_call_the_backends_prepare_restore_and_judge() {
    let run = include_str!("../src/run.rs");
    let driver = include_str!("../src/driver.rs");
    for (file, text) in [("run.rs", run), ("driver.rs", driver)] {
        for call in ["session.prepare(", "restore_trial(", "judge_trial("] {
            assert!(text.contains(call), "{file} no longer calls {call}");
        }
    }
    for forbidden in [
        "Command::new",
        "spawn(",
        "UnixListener",
        "kill(",
        "Director::start",
    ] {
        assert!(
            !driver.contains(forbidden),
            "the driver holds host code: {forbidden}"
        );
    }
}
