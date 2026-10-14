//! WASM backend tests. Guests are written in WAT so no toolchain is needed.
use anthill_core::bundle::Bundle;
use anthill_engine::config::{Budget, Config, TargetKind, TargetSpec};
use anthill_engine::engine::{Engine, RunRequest};
use anthill_engine::erased::Registry;
use anthill_engine::report::RunStatus;
use anthill_engine::EngineError;
use anthill_wasm::{RunResult, VmConfig, WasmBackend, WasmSettings, WasmVm};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Two "threads" each do read-then-write on a shared balance; `choose(2)` picks who steps next.
const LOST_UPDATE: &str = r#"
(module
  (import "anthill" "choose" (func $choose (param i32) (result i32)))
  (import "anthill" "fail" (func $fail (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "lost update")
  (global $bal (mut i32) (i32.const 100))
  (global $atmp (mut i32) (i32.const 0))
  (global $btmp (mut i32) (i32.const 0))
  (global $apc (mut i32) (i32.const 0))
  (global $bpc (mut i32) (i32.const 0))
  (func $step_a
    (if (i32.eqz (global.get $apc))
      (then (global.set $atmp (global.get $bal)) (global.set $apc (i32.const 1)) (return)))
    (if (i32.eq (global.get $apc) (i32.const 1))
      (then (global.set $bal (i32.add (global.get $atmp) (i32.const 10))) (global.set $apc (i32.const 2)))))
  (func $step_b
    (if (i32.eqz (global.get $bpc))
      (then (global.set $btmp (global.get $bal)) (global.set $bpc (i32.const 1)) (return)))
    (if (i32.eq (global.get $bpc) (i32.const 1))
      (then (global.set $bal (i32.add (global.get $btmp) (i32.const 10))) (global.set $bpc (i32.const 2)))))
  (func (export "run") (result i32)
    (block $done
      (loop $l
        (br_if $done (i32.and (i32.eq (global.get $apc) (i32.const 2)) (i32.eq (global.get $bpc) (i32.const 2))))
        (if (i32.eq (call $choose (i32.const 2)) (i32.const 0)) (then (call $step_a)) (else (call $step_b)))
        (br $l)))
    (if (i32.ne (global.get $bal) (i32.const 120)) (then (call $fail (i32.const 0) (i32.const 11))))
    (i32.const 0)))
"#;

/// The fix: the read-modify-write is a single step.
const ATOMIC: &str = r#"
(module
  (import "anthill" "choose" (func $choose (param i32) (result i32)))
  (import "anthill" "fail" (func $fail (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "lost update")
  (global $bal (mut i32) (i32.const 100))
  (global $adone (mut i32) (i32.const 0))
  (global $bdone (mut i32) (i32.const 0))
  (func (export "run") (result i32)
    (block $done
      (loop $l
        (br_if $done (i32.and (global.get $adone) (global.get $bdone)))
        (if (i32.eq (call $choose (i32.const 2)) (i32.const 0))
          (then (if (i32.eqz (global.get $adone)) (then (global.set $bal (i32.add (global.get $bal) (i32.const 10))) (global.set $adone (i32.const 1)))))
          (else (if (i32.eqz (global.get $bdone)) (then (global.set $bal (i32.add (global.get $bal) (i32.const 10))) (global.set $bdone (i32.const 1))))))
        (br $l)))
    (if (i32.ne (global.get $bal) (i32.const 120)) (then (call $fail (i32.const 0) (i32.const 11))))
    (i32.const 0)))
"#;

const COUNTER: &str = r#"
(module
  (import "anthill" "rand_u64" (func $rand (result i64)))
  (memory (export "memory") 1)
  (global $g (export "g") (mut i32) (i32.const 0))
  (func (export "bump") (result i32)
    (i32.store (i32.const 16) (i32.add (i32.load (i32.const 16)) (i32.const 1)))
    (global.set $g (i32.add (global.get $g) (i32.const 1)))
    (i32.load (i32.const 16)))
  (func (export "get") (result i32) (i32.load (i32.const 16)))
  (func (export "getg") (result i32) (global.get $g))
  (func (export "rnd") (result i64) (call $rand)))
"#;

fn cfg() -> VmConfig {
    VmConfig::default()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "anthill-wasm-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn engine_for(wat: &str, name: &str, budget: Budget) -> (Engine, PathBuf) {
    let dir = scratch(name);
    let path = dir.join("m.wat");
    std::fs::write(&path, wat).unwrap();
    engine_at(&path, budget).map(|e| (e, path)).unwrap()
}

fn engine_at(path: &std::path::Path, budget: Budget) -> Result<Engine, EngineError> {
    let spec = TargetSpec {
        name: "w".into(),
        description: String::new(),
        kind: TargetKind::Wasm {
            module: path.display().to_string(),
        },
        budget,
    };
    Ok(Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("w".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(WasmBackend::new(WasmSettings::default()))))
}

fn budget(trials: u64) -> Budget {
    Budget {
        max_steps: 1_000_000,
        max_trials: trials,
        wall_secs: 60,
    }
}

// ---- determinism ----
#[test]
fn the_same_seed_gives_identical_runs_and_different_seeds_differ() {
    let run = |seed| {
        let mut vm = WasmVm::new(LOST_UPDATE.as_bytes(), seed, vec![], cfg()).unwrap();
        let r = vm.run();
        (r, vm.choices().to_vec(), vm.trace_hash())
    };
    assert_eq!(run(7), run(7));
    let hashes: std::collections::HashSet<_> = (1..30).map(|s| run(s).2).collect();
    assert!(
        hashes.len() > 5,
        "seeds explore different schedules: {}",
        hashes.len()
    );
}

#[test]
fn golden_trace_hash_is_identical_on_every_platform() {
    // Recorded on macOS arm64 and verified on Linux x86-64: WASM execution does not depend on the host.
    let mut vm = WasmVm::new(LOST_UPDATE.as_bytes(), 7, vec![], cfg()).unwrap();
    let r = vm.run();
    assert_eq!(
        vm.trace_hash(),
        "01919384cb47b910",
        "result {r:?}, choices {:?}",
        vm.choices()
    );
}

// ---- finding, shrinking, replaying ----
#[test]
fn finds_the_lost_update_shrinks_it_and_replays_it() {
    let (e, _p) = engine_for(LOST_UPDATE, "find", budget(200));
    let out = e.run(&RunRequest::new("w", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    let f = out.failure.unwrap();
    assert_eq!(f.backend, "wasm");
    assert!(f.message.contains("lost update"), "{}", f.message);
    assert!(
        f.shrunk_len <= f.original_len && f.shrunk_len >= 2,
        "{} -> {}",
        f.original_len,
        f.shrunk_len
    );
    assert!(f.steps.iter().any(|s| s.label.contains("choose(2)")));
    let b = Bundle::decode(&f.bundle).unwrap();
    assert_eq!((b.backend.as_str(), b.target.as_str()), ("wasm", "w"));
    assert!(b.params.contains_key("module_hash"));
    let r = e.replay(&b).unwrap();
    assert!(r.reproduced);
    assert_eq!(r.steps, f.steps, "replay reproduces the identical trace");
}

#[test]
fn the_shrunk_trace_is_minimal_for_the_lost_update() {
    let (e, _p) = engine_for(LOST_UPDATE, "min", budget(300));
    let f = e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    let b = Bundle::decode(&f.bundle).unwrap();
    // Minimal: A reads, B reads, B writes; replay then defaults the remaining decisions to 0 (A writes last),
    // which overwrites B's update. Three explicit decisions are enough and two are not (those hang instead).
    assert_eq!(b.path.len(), 3, "path {:?}", b.path);
}

#[test]
fn the_atomic_version_passes_every_seed() {
    let (e, _p) = engine_for(ATOMIC, "atomic", budget(150));
    let out = e.run(&RunRequest::new("w", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Passed, "{out:?}");
    assert_eq!(out.trials, 150);
}

#[test]
fn doctor_confirms_the_host_is_deterministic() {
    let (e, _p) = engine_for(LOST_UPDATE, "doctor", budget(1));
    assert!(e.doctor("w", 10, 0).unwrap().ok);
}

// ---- failure modes of the guest ----
#[test]
fn an_infinite_loop_is_reported_as_a_hang_within_the_fuel_budget() {
    let wat = r#"(module (memory (export "memory") 1) (func (export "run") (result i32) (loop $l (br $l)) (i32.const 0)))"#;
    let (e, _p) = engine_for(
        wat,
        "hang",
        Budget {
            max_steps: 100_000,
            max_trials: 3,
            wall_secs: 30,
        },
    );
    let started = std::time::Instant::now();
    let out = e.run(&RunRequest::new("w", 1), &mut |_| true).unwrap();
    assert_eq!(out.status, RunStatus::Failed);
    assert!(
        out.failure
            .unwrap()
            .message
            .to_lowercase()
            .contains("instructions"),
        "mentions the budget"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn traps_and_nonzero_exits_are_violations() {
    let trap =
        r#"(module (memory (export "memory") 1) (func (export "run") (result i32) unreachable))"#;
    let (e, _p) = engine_for(trap, "trap", budget(2));
    let f = e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    assert!(f.message.to_lowercase().contains("trap"), "{}", f.message);
    let exit =
        r#"(module (memory (export "memory") 1) (func (export "run") (result i32) (i32.const 3)))"#;
    let (e, _p) = engine_for(exit, "exit", budget(2));
    assert!(e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap()
        .message
        .contains("exit code 3"));
}

#[test]
fn guests_cannot_grow_memory_past_the_limit() {
    let wat = r#"(module (memory (export "memory") 1)
      (func (export "run") (result i32)
        (if (result i32) (i32.eq (memory.grow (i32.const 4000)) (i32.const -1)) (then (i32.const 0)) (else (i32.const 1)))))"#;
    let (e, _p) = engine_for(wat, "mem", budget(1));
    assert_eq!(
        e.run(&RunRequest::new("w", 1), &mut |_| true)
            .unwrap()
            .status,
        RunStatus::Passed,
        "growth to 250 MiB must be denied"
    );
}

// ---- snapshot / fork ----
#[test]
fn snapshot_restore_and_fork_reproduce_state_exactly() {
    let mut vm = WasmVm::new(COUNTER.as_bytes(), 5, vec![], cfg()).unwrap();
    for _ in 0..3 {
        vm.call_i32("bump").unwrap();
    }
    let snap = vm.snapshot().unwrap();
    let r_first = vm.call_i64("rnd").unwrap();
    vm.call_i32("bump").unwrap();
    vm.call_i32("bump").unwrap();
    assert_eq!(vm.call_i32("get").unwrap(), 5);

    vm.restore(&snap).unwrap();
    assert_eq!(vm.call_i32("get").unwrap(), 3, "linear memory restored");
    assert_eq!(
        vm.call_i32("getg").unwrap(),
        3,
        "exported mutable globals restored"
    );
    assert_eq!(
        vm.call_i64("rnd").unwrap(),
        r_first,
        "host RNG state restored: the future repeats"
    );

    let mut forked = vm.fork(&snap).unwrap();
    assert_eq!(forked.call_i32("get").unwrap(), 3);
    forked.call_i32("bump").unwrap();
    assert_eq!(forked.call_i32("get").unwrap(), 4);
    assert_ne!(
        vm.call_i32("get").unwrap(),
        4,
        "the fork is independent of the original"
    );
}

// ---- inputs are validated ----
#[test]
fn bad_modules_are_clear_errors() {
    let (e, p) = engine_for("(module", "bad", budget(1));
    assert_eq!(
        e.run(&RunRequest::new("w", 1), &mut |_| true)
            .unwrap()
            .status,
        RunStatus::Error
    );
    let missing = engine_at(&p.with_file_name("nope.wasm"), budget(1)).unwrap();
    let out = missing
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Error);
    assert!(out.error.unwrap().contains("nope.wasm"));
    let unknown_import = r#"(module (import "env" "mystery" (func)) (memory (export "memory") 1) (func (export "run") (result i32) (i32.const 0)))"#;
    let (e, _p) = engine_for(unknown_import, "imp", budget(1));
    let msg = e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .error
        .unwrap();
    assert!(
        msg.contains("mystery") || msg.to_lowercase().contains("import"),
        "{msg}"
    );
    let no_run = r#"(module (memory (export "memory") 1))"#;
    let (e, _p) = engine_for(no_run, "norun", budget(1));
    assert!(e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .error
        .unwrap()
        .contains("run"));
}

#[test]
fn an_oversized_module_is_rejected_before_compiling() {
    let dir = scratch("big");
    let p = dir.join("big.wasm");
    std::fs::write(&p, vec![0u8; 17 * 1024 * 1024]).unwrap();
    let out = engine_at(&p, budget(1))
        .unwrap()
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap();
    assert_eq!(out.status, RunStatus::Error);
    assert!(out.error.unwrap().contains("too large"));
}

#[test]
fn replay_refuses_a_bundle_recorded_against_a_different_module() {
    let (e, p) = engine_for(LOST_UPDATE, "swap", budget(200));
    let f = e
        .run(&RunRequest::new("w", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    std::fs::write(&p, ATOMIC).unwrap(); // the program changed since the bundle was recorded
    let err = e.replay(&Bundle::decode(&f.bundle).unwrap()).unwrap_err();
    assert!(
        matches!(err, EngineError::InvalidBundle(m) if m.contains("module")),
        "must not silently replay against another program"
    );
}

#[test]
fn exploration_can_be_cancelled() {
    let (e, _p) = engine_for(ATOMIC, "cancel", budget(10_000));
    let mut n = 0;
    let out = e
        .run(&RunRequest::new("w", 1), &mut |_| {
            n += 1;
            n < 4
        })
        .unwrap();
    assert_eq!(out.error.as_deref(), Some("cancelled"));
}

#[test]
fn the_vm_reports_run_results_directly() {
    let mut vm = WasmVm::new(LOST_UPDATE.as_bytes(), 1, vec![0, 1, 0, 1], cfg()).unwrap();
    // forced choices A,B,A,B = both read then both write: the classic lost update
    assert!(matches!(vm.run(), RunResult::Violation(m) if m.contains("lost update")));
}
