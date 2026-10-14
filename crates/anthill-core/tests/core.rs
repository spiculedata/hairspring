use anthill_core::bundle::{Bundle, BundleError};
use anthill_core::demo::{Action, LockLease};
use anthill_core::doctor::doctor;
use anthill_core::explore::{branching, random_restarts, Config};
use anthill_core::replay::{replay, ReplayError};
use anthill_core::rng::Rng;
use anthill_core::shrink::shrink;
use anthill_core::{stable_id, ActionId, Sim};
use std::collections::HashMap;

fn id(a: Action) -> ActionId {
    LockLease::action_id(&a)
}

fn minimal_split_brain() -> Vec<ActionId> {
    use Action::*;
    [
        Acquire(0),
        Deliver(0),
        Tick,
        Tick,
        Tick,
        Acquire(1),
        Deliver(1),
        Write(0),
    ]
    .map(id)
    .to_vec()
}

// ---- rng ----
#[test]
fn rng_matches_splitmix64_reference_vector() {
    let mut r = Rng::new(0);
    assert_eq!(r.next_u64(), 0xE220A8397B1DCDAF);
    assert_eq!(r.next_u64(), 0x6E789E6AA1B965F4);
}
#[test]
fn rng_below_stays_in_range() {
    let mut r = Rng::new(7);
    assert!((0..1000).all(|_| r.below(5) < 5));
}

// ---- stable ids ----
#[test]
fn action_ids_are_unique_across_the_action_space() {
    use Action::*;
    let all = [
        Tick,
        Acquire(0),
        Acquire(1),
        Deliver(0),
        Deliver(1),
        Drop(0),
        Drop(1),
        Pause(0),
        Pause(1),
        Resume(0),
        Resume(1),
        Write(0),
        Write(1),
    ];
    let mut ids: Vec<_> = all.iter().map(|a| id(*a)).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all.len());
    assert_eq!(stable_id(&[1, 2]), stable_id(&[1, 2]));
    assert_ne!(stable_id(&[1, 2]), stable_id(&[2, 1]));
}

// ---- replay ----
#[test]
fn replay_of_minimal_path_reaches_violation() {
    let r = replay(&LockLease::new(), &minimal_split_brain()).unwrap();
    assert!(r.state.violated());
    assert_eq!(r.applied, 8);
}
#[test]
fn replay_rejects_action_that_is_not_enabled() {
    let bad = vec![id(Action::Write(0))];
    assert_eq!(
        replay(&LockLease::new(), &bad).err(),
        Some(ReplayError::NotEnabled {
            step: 0,
            id: bad[0]
        })
    );
}
#[test]
fn apply_does_not_mutate_the_original_state() {
    let root = LockLease::new();
    let _next = root.apply(Action::Tick);
    assert_eq!(root.time, 0);
}

// ---- shrink ----
#[test]
fn shrink_removes_noise_down_to_the_minimal_trace() {
    use Action::*;
    let noisy: Vec<ActionId> = [
        Tick,
        Acquire(0),
        Deliver(0),
        Pause(0),
        Resume(0),
        Pause(0),
        Resume(0),
        Tick,
        Tick,
        Tick,
        Acquire(1),
        Deliver(1),
        Pause(1),
        Resume(1),
        Write(0),
    ]
    .map(id)
    .to_vec();
    assert!(replay(&LockLease::new(), &noisy).unwrap().state.violated());
    let small = shrink(&LockLease::new(), &noisy);
    assert_eq!(small.len(), 8, "minimal split-brain needs exactly 8 steps");
    assert!(replay(&LockLease::new(), &small).unwrap().state.violated());
}
#[test]
fn shrink_is_idempotent() {
    let once = shrink(&LockLease::new(), &minimal_split_brain());
    assert_eq!(shrink(&LockLease::new(), &once), once);
}

// ---- explore ----
#[test]
fn both_explorers_find_the_bug_and_return_replayable_paths() {
    let cfg = Config {
        seed: 3,
        ..Config::default()
    };
    for f in [
        random_restarts(&LockLease::new(), &cfg),
        branching(&LockLease::new(), &cfg),
    ] {
        let f = f.expect("bug should be found within budget");
        assert!(replay(&LockLease::new(), &f.path).unwrap().state.violated());
    }
}
#[test]
fn exploration_is_deterministic_for_a_given_seed() {
    let cfg = Config {
        seed: 11,
        ..Config::default()
    };
    let a = branching(&LockLease::new(), &cfg).unwrap();
    let b = branching(&LockLease::new(), &cfg).unwrap();
    assert_eq!((a.path, a.steps_executed), (b.path, b.steps_executed));
}
#[test]
fn branching_beats_random_restarts_on_median_steps() {
    let median = |f: fn(&LockLease, &Config) -> Option<anthill_core::explore::Failure>| {
        let mut v: Vec<u64> = (1..=100u64)
            .map(|s| {
                f(
                    &LockLease::new(),
                    &Config {
                        seed: s,
                        ..Config::default()
                    },
                )
                .unwrap()
                .steps_executed
            })
            .collect();
        v.sort();
        v[v.len() / 2]
    };
    assert!(median(branching::<LockLease>) < median(random_restarts::<LockLease>));
}

// ---- doctor ----
#[test]
fn doctor_passes_for_a_deterministic_sim() {
    let a = doctor(&LockLease::new(), 42, 50).unwrap();
    let b = doctor(&LockLease::new(), 42, 50).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, doctor(&LockLease::new(), 43, 50).unwrap());
}

#[derive(Clone)]
struct Leaky {
    n: u32,
}
impl Sim for Leaky {
    type Action = u64;
    // BUG under test: iterates a freshly built HashMap (new RandomState per call).
    fn enabled(&self) -> Vec<u64> {
        if self.n > 5 {
            return vec![];
        }
        let m: HashMap<u64, u64> = (0..16).map(|i| (i, i)).collect();
        m.keys().copied().collect()
    }
    fn action_id(a: &u64) -> ActionId {
        *a
    }
    fn apply(&self, _a: u64) -> Self {
        Leaky { n: self.n + 1 }
    }
    fn signature(&self) -> u64 {
        self.n as u64
    }
    fn violated(&self) -> bool {
        false
    }
}
#[test]
fn doctor_catches_hashmap_order_leak() {
    let root = Leaky { n: 0 };
    let d = doctor(&root, 1, 5).expect_err("must detect divergence");
    assert_eq!(d.step, 0);
}

// ---- bundle ----
#[test]
fn bundle_round_trips_with_target_backend_and_params() {
    let b = Bundle::new(9, "abc123", minimal_split_brain())
        .with_target("lock-lease")
        .with_backend("sim")
        .with_param("sched_seed", "42")
        .with_param("note", "has = equals, 100% and\nnewline\r!");
    let text = b.encode();
    assert!(text.starts_with("anthill-bundle v2\n"));
    assert_eq!(Bundle::decode(&text).unwrap(), b);
}
#[test]
fn bundle_decodes_legacy_v1_with_defaults() {
    let v1 = "anthill-bundle v1\nseed=7\nbuild=x\npath=0000000000000001,0000000000000002\n";
    let b = Bundle::decode(v1).unwrap();
    assert_eq!((b.seed, b.build.as_str(), b.path.len()), (7, "x", 2));
    assert_eq!((b.target.as_str(), b.backend.as_str()), ("", "sim"));
}
#[test]
fn bundle_rejects_bad_input() {
    assert_eq!(Bundle::decode("nope"), Err(BundleError::BadHeader));
    assert_eq!(
        Bundle::decode("anthill-bundle v2\nbuild=x\npath=\n"),
        Err(BundleError::MissingField("seed"))
    );
    assert!(matches!(
        Bundle::decode("anthill-bundle v2\nseed=zz\nbuild=x\npath=\n"),
        Err(BundleError::BadNumber(_))
    ));
    assert!(matches!(
        Bundle::decode("anthill-bundle v2\nseed=1\nbuild=x\npath=zz\n"),
        Err(BundleError::BadNumber(_))
    ));
}
#[test]
fn bundle_with_empty_path_round_trips() {
    let b = Bundle::new(0, "x", vec![]);
    assert_eq!(Bundle::decode(&b.encode()).unwrap(), b);
}
#[test]
fn bundle_decode_ignores_unknown_fields_for_forward_compat() {
    let t = "anthill-bundle v2\nseed=1\nbuild=x\nfuture=thing\npath=\n";
    assert_eq!(Bundle::decode(t).unwrap().seed, 1);
}
