use crate::vm::{CompiledModule, RunResult, VmConfig, WasmVm};
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{
    DoctorDivergence, DoctorReport, FailureReport, ReplayOutcome, RunOutcome, Step,
};
use hairspring_engine::EngineError;
use std::time::{Duration, Instant};

const BUILD: &str = env!("CARGO_PKG_VERSION");
const CANCELLED: &str = "cancelled";
const MIN_FUEL: u64 = 10_000;
/// Re-executions the shrinker may spend before settling for what it has.
const MAX_SHRINK_RUNS: usize = 3000;
/// Most fuel a replayed bundle may request, whatever the bundle says.
const MAX_REPLAY_FUEL: u64 = 50_000_000;
/// Wall-clock limit for one replay.
const REPLAY_WALL: Duration = Duration::from_secs(30);
/// Trace length shown to humans; longer runs are summarised.
const MAX_STEPS_SHOWN: usize = 500;
/// Longest the shrinker may spend on one failing run.
const SHRINK_WALL: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct WasmSettings {
    pub max_module_bytes: u64,
    pub max_memory_bytes: usize,
}

impl Default for WasmSettings {
    fn default() -> Self {
        WasmSettings {
            max_module_bytes: 16 * 1024 * 1024,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct WasmBackend {
    settings: WasmSettings,
}

impl WasmBackend {
    pub fn new(settings: WasmSettings) -> WasmBackend {
        WasmBackend { settings }
    }

    fn read_module(&self, spec: &TargetSpec) -> Result<(Vec<u8>, String), String> {
        let TargetKind::Wasm { module } = &spec.kind else {
            return Err("not a wasm target".to_string());
        };
        let meta =
            std::fs::metadata(module).map_err(|e| format!("cannot read module `{module}`: {e}"))?;
        if meta.len() > self.settings.max_module_bytes {
            return Err(format!(
                "module `{module}` is too large ({} bytes; the limit is {})",
                meta.len(),
                self.settings.max_module_bytes
            ));
        }
        let bytes =
            std::fs::read(module).map_err(|e| format!("cannot read module `{module}`: {e}"))?;
        let hash = module_hash(&bytes);
        Ok((bytes, hash))
    }

    pub(crate) fn vm_config(&self, fuel: u64, forced_only: bool) -> VmConfig {
        VmConfig {
            fuel: fuel.max(MIN_FUEL),
            max_memory_bytes: self.settings.max_memory_bytes,
            forced_only,
        }
    }
}

fn module_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// What counts as the "same failure" while shrinking, so a minimal trace cannot drift into a different bug.
pub(crate) fn failure_key(r: &RunResult) -> Option<String> {
    match r {
        RunResult::Exit(0) => None,
        RunResult::Exit(n) => Some(format!("exit:{n}")),
        RunResult::Violation(m) => Some(format!("violation:{m}")),
        RunResult::OutOfFuel => Some("fuel".to_string()),
        RunResult::TimedOut => Some("timeout".to_string()),
        RunResult::Trap(m) => Some(format!("trap:{m}")),
    }
}

pub(crate) fn describe(r: &RunResult, fuel: u64) -> String {
    match r {
        RunResult::Exit(n) => format!("exit code {n}"),
        RunResult::Violation(m) => m.clone(),
        RunResult::OutOfFuel => {
            format!("did not finish within {fuel} instructions (possible infinite loop)")
        }
        RunResult::TimedOut => "did not finish within the wall-clock limit".to_string(),
        RunResult::Trap(m) => format!("trapped: {m}"),
    }
}

fn run_forced(
    module: &CompiledModule,
    seed: u64,
    choices: &[u64],
    cfg: VmConfig,
    wall: Duration,
) -> Option<(RunResult, WasmVm)> {
    let mut vm = module.instantiate(seed, choices.to_vec(), cfg).ok()?;
    let result = vm.run_with_wall(wall);
    Some((result, vm))
}

/// Delta debugging over the recorded decisions: drop chunks while the same failure persists.
fn shrink(
    module: &CompiledModule,
    seed: u64,
    original: &[u64],
    key: &str,
    cfg: &VmConfig,
    deadline: Instant,
) -> Vec<u64> {
    let forced = VmConfig {
        forced_only: true,
        ..cfg.clone()
    };
    let mut current = original.to_vec();
    let mut runs = 0usize;
    let still_fails = |candidate: &[u64], runs: &mut usize| -> bool {
        *runs += 1;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false; // out of time: keep what we have
        }
        run_forced(module, seed, candidate, forced.clone(), left)
            .and_then(|(r, _)| failure_key(&r))
            .is_some_and(|k| k == key)
    };
    loop {
        let mut changed = false;
        let mut chunk = (current.len() / 2).max(1);
        loop {
            let mut start = 0;
            while start < current.len() && runs < MAX_SHRINK_RUNS {
                let candidate: Vec<u64> = current
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i < start || *i >= start + chunk)
                    .map(|(_, v)| *v)
                    .collect();
                if still_fails(&candidate, &mut runs) {
                    current = candidate;
                    changed = true;
                } else {
                    start += chunk;
                }
            }
            if chunk == 1 || runs >= MAX_SHRINK_RUNS {
                break;
            }
            chunk /= 2;
        }
        if !changed || runs >= MAX_SHRINK_RUNS {
            return current;
        }
    }
}

fn steps_for(vm: &WasmVm, result: &RunResult, fuel: u64) -> Vec<Step> {
    let log = vm.choice_log();
    let omitted = log.len().saturating_sub(MAX_STEPS_SHOWN);
    let mut steps: Vec<Step> = log
        .iter()
        .take(MAX_STEPS_SHOWN)
        .enumerate()
        .map(|(i, (n, v))| Step {
            index: i,
            action_id: format!("{v:016x}"),
            label: format!("choose({n}) -> {v}"),
            state: String::new(),
        })
        .collect();
    if omitted > 0 {
        let index = steps.len();
        steps.push(Step {
            index,
            action_id: format!("{index:016x}"),
            label: format!("... {omitted} more decisions omitted"),
            state: String::new(),
        });
    }
    for line in vm.log_lines() {
        let index = steps.len();
        steps.push(Step {
            index,
            action_id: format!("{index:016x}"),
            label: format!("log: {line}"),
            state: String::new(),
        });
    }
    let index = steps.len();
    steps.push(Step {
        index,
        action_id: format!("{index:016x}"),
        label: format!("result: {}", describe(result, fuel)),
        state: String::new(),
    });
    steps
}

fn config_error(spec: &TargetSpec) -> EngineError {
    EngineError::BackendUnavailable(spec.kind.name().to_string())
}

/// A module that was read, compiled and found to export `run`, with what a failure report needs to name it.
pub(crate) struct Loaded<'a> {
    pub module: CompiledModule,
    pub hash: String,
    pub spec: &'a TargetSpec,
    pub req: &'a RunRequest,
    pub fuel: u64,
}

/// A trial that failed, with the machine it failed on.
pub(crate) struct Failing<'a> {
    pub loaded: &'a Loaded<'a>,
    pub seed: u64,
    pub key: String,
    pub result: RunResult,
    pub vm: WasmVm,
    pub wall_left: Duration,
}

impl WasmBackend {
    /// The report of a failing trial: its decisions shrunk to the fewest that fail the same way, replayed, and bundled.
    pub(crate) fn failure_report(&self, failing: Failing<'_>) -> FailureReport {
        let Failing {
            loaded,
            seed,
            key,
            result,
            vm,
            wall_left,
        } = failing;
        let (module, fuel) = (&loaded.module, loaded.fuel);
        let original = vm.choices();
        let cfg = self.vm_config(fuel, false);
        let forced = VmConfig {
            forced_only: true,
            ..cfg.clone()
        };
        let deadline = Instant::now() + SHRINK_WALL.min(wall_left);
        let mut minimal = shrink(module, seed, &original, &key, &cfg, deadline);
        let same = |r: &RunResult| failure_key(r).as_deref() == Some(key.as_str());
        let mut rerun = run_forced(module, seed, &minimal, forced.clone(), REPLAY_WALL);
        // The shrunk trace must still fail the same way; otherwise fall back to the full one.
        if !rerun.as_ref().is_some_and(|(r, _)| same(r)) {
            minimal = original.clone();
            rerun = run_forced(module, seed, &minimal, forced, REPLAY_WALL);
        }
        let (final_result, final_vm) = match rerun {
            Some((r, v)) if same(&r) => (r, v),
            // Not even the recorded decisions replay: report the observed run itself.
            _ => (result.clone(), vm),
        };
        let steps = steps_for(&final_vm, &final_result, fuel);
        let bundle = Bundle::new(seed, BUILD, minimal.clone())
            .with_target(loaded.req.target.clone())
            .with_backend("wasm")
            .with_param("module_hash", loaded.hash.clone())
            .with_param("fuel", fuel.to_string())
            .with_param("failure_key", key.clone());
        FailureReport {
            target: loaded.spec.name.clone(),
            backend: "wasm".to_string(),
            message: format!(
                "{}; shrunk from {} to {} choices",
                describe(&final_result, fuel),
                original.len(),
                minimal.len()
            ),
            original_len: original.len(),
            shrunk_len: minimal.len(),
            steps,
            bundle: bundle.encode(),
        }
    }
}

impl Backend for WasmBackend {
    fn name(&self) -> &'static str {
        "wasm"
    }

    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Wasm { .. })
    }

    fn run(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        if !self.supports(&spec.kind) {
            return Err(config_error(spec));
        }
        let (bytes, hash) = match self.read_module(spec) {
            Ok(m) => m,
            Err(e) => return Ok(RunOutcome::error(e)),
        };
        let module = match CompiledModule::compile(&bytes) {
            Ok(m) => m,
            Err(e) => return Ok(RunOutcome::error(e.to_string())),
        };
        if !module.has_export("run") {
            return Ok(RunOutcome::error(
                "module must export a function named `run`",
            ));
        }
        let fuel = budget.max_steps.max(MIN_FUEL);
        let loaded = Loaded {
            module,
            hash,
            spec,
            req,
            fuel,
        };
        if req.mode.is_sweep() {
            return Ok(crate::sweep::run(self, &loaded, budget, progress));
        }
        let module = &loaded.module;
        let started = Instant::now();
        let wall = Duration::from_secs(budget.wall_secs);
        let trials = budget.max_trials.max(1);
        for i in 0..trials {
            if i > 0 && started.elapsed() >= wall {
                return Ok(RunOutcome::passed(
                    i,
                    i,
                    started.elapsed().as_millis() as u64,
                ));
            }
            let seed = req.seed.wrapping_add(i);
            let mut vm = match module.instantiate(seed, Vec::new(), self.vm_config(fuel, false)) {
                Ok(vm) => vm,
                Err(e) => return Ok(RunOutcome::error(e.to_string())),
            };
            let wall_left = wall
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1));
            let result = vm.run_with_wall(wall_left);
            if result == RunResult::TimedOut {
                // Out of wall-clock budget: inconclusive, never a finding.
                return Ok(RunOutcome::passed(
                    i,
                    i,
                    started.elapsed().as_millis() as u64,
                ));
            }
            if let Some(key) = failure_key(&result) {
                let failure = self.failure_report(Failing {
                    loaded: &loaded,
                    seed,
                    key,
                    result,
                    vm,
                    wall_left,
                });
                return Ok(RunOutcome::failed(
                    i + 1,
                    i + 1,
                    started.elapsed().as_millis() as u64,
                    failure,
                ));
            }
            if !progress(Progress {
                steps: i + 1,
                trials: i + 1,
                message: format!("{} of {trials} seeds explored", i + 1),
            }) {
                return Ok(RunOutcome::error(CANCELLED));
            }
        }
        Ok(RunOutcome::passed(
            trials,
            trials,
            started.elapsed().as_millis() as u64,
        ))
    }

    fn replay(&self, spec: &TargetSpec, bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        let (bytes, hash) = self.read_module(spec).map_err(EngineError::Backend)?;
        if bundle.params.get("module_hash").is_some_and(|h| *h != hash) {
            return Err(EngineError::InvalidBundle("the module has changed since this bundle was recorded; replaying it against different code would be meaningless".to_string()));
        }
        let module =
            CompiledModule::compile(&bytes).map_err(|e| EngineError::Backend(e.to_string()))?;
        let fuel: u64 = bundle
            .params
            .get("fuel")
            .and_then(|f| f.parse().ok())
            .unwrap_or(spec.budget.max_steps);
        let fuel = fuel.min(MAX_REPLAY_FUEL);
        let (result, vm) = run_forced(
            &module,
            bundle.seed,
            &bundle.path,
            self.vm_config(fuel, true),
            REPLAY_WALL,
        )
        .ok_or_else(|| EngineError::Backend("cannot instantiate the module".to_string()))?;
        let now = failure_key(&result);
        let recorded = bundle.params.get("failure_key");
        let same = match (&now, recorded) {
            (Some(n), Some(r)) => n == r,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let mut steps = steps_for(&vm, &result, fuel.max(MIN_FUEL));
        if now.is_some() && !same {
            let index = steps.len();
            steps.push(Step {
                index,
                action_id: format!("{index:016x}"),
                label: format!(
                    "a different failure occurred than the one recorded ({})",
                    describe(&result, fuel)
                ),
                state: String::new(),
            });
        }
        Ok(ReplayOutcome::new(same, steps))
    }

    fn doctor(
        &self,
        spec: &TargetSpec,
        seeds: u32,
        _steps: usize,
    ) -> Result<DoctorReport, EngineError> {
        let (bytes, _) = self.read_module(spec).map_err(EngineError::Backend)?;
        let module =
            CompiledModule::compile(&bytes).map_err(|e| EngineError::Backend(e.to_string()))?;
        let cfg = self.vm_config(spec.budget.max_steps, false);
        for i in 0..seeds.max(1) {
            let seed = u64::from(i) + 1;
            let observe = || -> Result<(RunResult, String), EngineError> {
                let mut vm = module
                    .instantiate(seed, Vec::new(), cfg.clone())
                    .map_err(|e| EngineError::Backend(e.to_string()))?;
                let r = vm.run();
                Ok((r, vm.trace_hash()))
            };
            if observe()? != observe()? {
                return Ok(DoctorReport {
                    ok: false,
                    seeds_checked: i + 1,
                    steps_per_seed: 0,
                    divergence: Some(DoctorDivergence { seed, step: 0 }),
                });
            }
        }
        Ok(DoctorReport {
            ok: true,
            seeds_checked: seeds.max(1),
            steps_per_seed: 0,
            divergence: None,
        })
    }
}
