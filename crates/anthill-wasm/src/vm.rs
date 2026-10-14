use anthill_core::rng::Rng;
use std::fmt;
use wasmtime::{
    Caller, Config, Engine, Extern, Global, Instance, Linker, Memory, Module, Mutability, Store,
    StoreLimits, StoreLimitsBuilder, Trap, Val,
};

/// Separates the stream that answers `choose` from the one behind `rand_u64`, so replaying with forced
/// decisions leaves the guest's random numbers exactly as they were in the original run.
const CHOICE_SALT: u64 = 0xC401_CE5A_17D3_E551;
/// Most decisions a guest may make in one run; the log of them must stay bounded.
const MAX_DECISIONS: usize = 100_000;
const MAX_TABLE_ELEMENTS: usize = 10_000;
/// Epoch deadline meaning "never": large, but far enough from `u64::MAX` not to overflow when added.
const NO_DEADLINE: u64 = 1 << 62;
const WATCHDOG_TICK: std::time::Duration = std::time::Duration::from_millis(5);
const MAX_LOG_LINES: usize = 200;
const MAX_MESSAGE_BYTES: usize = 200;
const WASM_PAGE: usize = 65_536;

#[derive(Debug)]
pub enum WasmError {
    Compile(String),
    Instantiate(String),
    MissingExport(String),
    Runtime(String),
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WasmError::Compile(m) => write!(f, "cannot compile module: {m}"),
            WasmError::Instantiate(m) => write!(f, "cannot instantiate module: {m}"),
            WasmError::MissingExport(n) => write!(f, "module must export `{n}`"),
            WasmError::Runtime(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for WasmError {}

#[derive(Debug, Clone)]
pub struct VmConfig {
    /// Instruction budget for the whole run; exhausting it is reported as a hang.
    pub fuel: u64,
    pub max_memory_bytes: usize,
    /// When true, choices beyond the forced list are 0 instead of random (replay and shrinking).
    pub forced_only: bool,
}

impl Default for VmConfig {
    fn default() -> Self {
        VmConfig {
            fuel: 10_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
            forced_only: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResult {
    Exit(i32),
    Violation(String),
    OutOfFuel,
    /// The wall-clock watchdog fired (only with [`WasmVm::run_with_wall`]).
    TimedOut,
    Trap(String),
}

struct HostState {
    rng: Rng,
    choice_rng: Rng,
    forced: Vec<u64>,
    forced_only: bool,
    cursor: usize,
    choices: Vec<(u32, u64)>,
    log: Vec<String>,
    violation: Option<String>,
    clock_ms: u64,
    limits: StoreLimits,
}

#[derive(Clone)]
struct HostSnapshot {
    rng: Rng,
    choice_rng: Rng,
    forced: Vec<u64>,
    forced_only: bool,
    fuel: u64,
    cursor: usize,
    choices: Vec<(u32, u64)>,
    log: Vec<String>,
    clock_ms: u64,
}

#[derive(Clone)]
enum GlobalValue {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

/// A point-in-time copy of everything a guest can observe: linear memory, exported mutable
/// globals, and the host's randomness, choice cursor and clock. Tables and unexported globals
/// are not captured; guests meant for snapshotting should keep their state in memory.
#[derive(Clone)]
pub struct Snapshot {
    memory: Vec<u8>,
    globals: Vec<(String, GlobalValue)>,
    host: HostSnapshot,
}

/// A compiled module that can be instantiated many times (cheap) with different seeds.
#[derive(Clone)]
pub struct CompiledModule {
    engine: Engine,
    module: Module,
}

fn engine() -> Result<Engine, WasmError> {
    let mut c = Config::new();
    c.consume_fuel(true);
    // Fuel counts instructions, not the time spent inside host-side operations such as memory.fill;
    // epoch interruption lets a watchdog bound real time too.
    c.epoch_interruption(true);
    // The two documented sources of nondeterminism in core WebAssembly.
    c.cranelift_nan_canonicalization(true);
    c.relaxed_simd_deterministic(true);
    Engine::new(&c).map_err(|e| WasmError::Compile(e.to_string()))
}

impl CompiledModule {
    /// Compiles `.wasm` bytes or `.wat` text.
    pub fn compile(bytes: &[u8]) -> Result<CompiledModule, WasmError> {
        let engine = engine()?;
        let module = Module::new(&engine, bytes).map_err(|e| WasmError::Compile(e.to_string()))?;
        Ok(CompiledModule { engine, module })
    }

    pub fn has_export(&self, name: &str) -> bool {
        self.module.exports().any(|e| e.name() == name)
    }

    pub fn instantiate(
        &self,
        seed: u64,
        forced: Vec<u64>,
        cfg: VmConfig,
    ) -> Result<WasmVm, WasmError> {
        let state = HostState {
            rng: Rng::new(seed),
            choice_rng: Rng::new(seed ^ CHOICE_SALT),
            forced,
            forced_only: cfg.forced_only,
            cursor: 0,
            choices: Vec::new(),
            log: Vec::new(),
            violation: None,
            clock_ms: 0,
            limits: StoreLimitsBuilder::new()
                .memory_size(cfg.max_memory_bytes)
                .table_elements(MAX_TABLE_ELEMENTS)
                .tables(1)
                .memories(1)
                .instances(1)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(cfg.fuel)
            .map_err(|e| WasmError::Runtime(e.to_string()))?;
        store.set_epoch_deadline(NO_DEADLINE);
        let linker = host_linker(&self.engine)?;
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| WasmError::Instantiate(e.to_string()))?;
        Ok(WasmVm {
            compiled: self.clone(),
            store,
            instance,
            cfg,
        })
    }
}

fn guest_bytes(c: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Vec<u8> {
    let len = (len.max(0) as usize).min(MAX_MESSAGE_BYTES);
    let Some(Extern::Memory(mem)) = c.get_export("memory") else {
        return Vec::new();
    };
    let data = mem.data(&*c);
    let start = ptr.max(0) as usize;
    data.get(start..start.saturating_add(len))
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}

fn host_linker(engine: &Engine) -> Result<Linker<HostState>, WasmError> {
    let mut l: Linker<HostState> = Linker::new(engine);
    let err = |e: wasmtime::Error| WasmError::Instantiate(e.to_string());
    l.func_wrap(
        "anthill",
        "choose",
        |mut c: Caller<'_, HostState>, n: i32| -> wasmtime::Result<i32> {
            let n = n.max(1) as u32;
            let s = c.data_mut();
            if s.choices.len() >= MAX_DECISIONS {
                s.violation = Some(format!(
                    "made more than {MAX_DECISIONS} decisions (possible infinite loop)"
                ));
                return Err(wasmtime::Error::msg("anthill: too many decisions"));
            }
            let v = if s.cursor < s.forced.len() {
                s.forced[s.cursor] % u64::from(n)
            } else if s.forced_only {
                0
            } else {
                s.choice_rng.below(n as usize) as u64
            };
            s.cursor += 1;
            s.choices.push((n, v));
            Ok(v as i32)
        },
    )
    .map_err(err)?;
    l.func_wrap(
        "anthill",
        "rand_u64",
        |mut c: Caller<'_, HostState>| -> i64 { c.data_mut().rng.next_u64() as i64 },
    )
    .map_err(err)?;
    l.func_wrap("anthill", "now_ms", |c: Caller<'_, HostState>| -> i64 {
        c.data().clock_ms as i64
    })
    .map_err(err)?;
    l.func_wrap(
        "anthill",
        "sleep_ms",
        |mut c: Caller<'_, HostState>, ms: i64| {
            let s = c.data_mut();
            s.clock_ms = s.clock_ms.saturating_add(ms.max(0) as u64);
        },
    )
    .map_err(err)?;
    l.func_wrap(
        "anthill",
        "log",
        |mut c: Caller<'_, HostState>, ptr: i32, len: i32| {
            let text = String::from_utf8_lossy(&guest_bytes(&mut c, ptr, len)).into_owned();
            let s = c.data_mut();
            if s.log.len() < MAX_LOG_LINES {
                s.log.push(text);
            }
        },
    )
    .map_err(err)?;
    l.func_wrap(
        "anthill",
        "fail",
        |mut c: Caller<'_, HostState>, ptr: i32, len: i32| -> wasmtime::Result<()> {
            let text = String::from_utf8_lossy(&guest_bytes(&mut c, ptr, len)).into_owned();
            c.data_mut().violation = Some(text);
            Err(wasmtime::Error::msg("anthill: property violated"))
        },
    )
    .map_err(err)?;
    Ok(l)
}

pub struct WasmVm {
    compiled: CompiledModule,
    store: Store<HostState>,
    instance: Instance,
    cfg: VmConfig,
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h = (*h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
}

impl WasmVm {
    pub fn new(
        bytes: &[u8],
        seed: u64,
        forced: Vec<u64>,
        cfg: VmConfig,
    ) -> Result<WasmVm, WasmError> {
        CompiledModule::compile(bytes)?.instantiate(seed, forced, cfg)
    }

    fn memory(&mut self) -> Option<Memory> {
        self.instance.get_memory(&mut self.store, "memory")
    }

    /// Runs the guest's `run` export to completion.
    pub fn run(&mut self) -> RunResult {
        let Some(func) = self.instance.get_func(&mut self.store, "run") else {
            return RunResult::Trap("no `run` export".to_string());
        };
        let outcome: Result<i32, wasmtime::Error> = match func.typed::<(), i32>(&self.store) {
            Ok(typed) => typed.call(&mut self.store, ()),
            Err(_) => match func.typed::<(), ()>(&self.store) {
                Ok(typed) => typed.call(&mut self.store, ()).map(|()| 0),
                Err(e) => return RunResult::Trap(format!("`run` has the wrong signature: {e}")),
            },
        };
        if let Some(msg) = &self.store.data().violation {
            return RunResult::Violation(msg.clone());
        }
        match outcome {
            Ok(code) => RunResult::Exit(code),
            Err(e) if e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) => RunResult::OutOfFuel,
            Err(e) if e.downcast_ref::<Trap>() == Some(&Trap::Interrupt) => RunResult::TimedOut,
            Err(e) => RunResult::Trap(e.to_string().lines().next().unwrap_or("trap").to_string()),
        }
    }

    /// Like [`WasmVm::run`] but also stops the guest after `wall` of real time.
    pub fn run_with_wall(&mut self, wall: std::time::Duration) -> RunResult {
        use std::sync::atomic::{AtomicBool, Ordering};
        self.store.set_epoch_deadline(1);
        let engine = self.compiled.engine.clone();
        let done = std::sync::Arc::new(AtomicBool::new(false));
        let watchdog_done = done.clone();
        let watchdog = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            while !watchdog_done.load(Ordering::SeqCst) {
                if started.elapsed() >= wall {
                    engine.increment_epoch();
                    return;
                }
                std::thread::sleep(WATCHDOG_TICK);
            }
        });
        let result = self.run();
        done.store(true, Ordering::SeqCst);
        let _ = watchdog.join();
        self.store.set_epoch_deadline(NO_DEADLINE);
        result
    }

    pub fn call_i32(&mut self, name: &str) -> Result<i32, WasmError> {
        let f = self
            .instance
            .get_func(&mut self.store, name)
            .ok_or_else(|| WasmError::MissingExport(name.to_string()))?;
        let typed = f
            .typed::<(), i32>(&self.store)
            .map_err(|e| WasmError::Runtime(e.to_string()))?;
        typed
            .call(&mut self.store, ())
            .map_err(|e| WasmError::Runtime(e.to_string()))
    }

    pub fn call_i64(&mut self, name: &str) -> Result<i64, WasmError> {
        let f = self
            .instance
            .get_func(&mut self.store, name)
            .ok_or_else(|| WasmError::MissingExport(name.to_string()))?;
        let typed = f
            .typed::<(), i64>(&self.store)
            .map_err(|e| WasmError::Runtime(e.to_string()))?;
        typed
            .call(&mut self.store, ())
            .map_err(|e| WasmError::Runtime(e.to_string()))
    }

    /// The decisions made so far (values only).
    pub fn choices(&self) -> Vec<u64> {
        self.store.data().choices.iter().map(|(_, v)| *v).collect()
    }

    /// The decisions made so far as `(range, value)`.
    pub fn choice_log(&self) -> Vec<(u32, u64)> {
        self.store.data().choices.clone()
    }

    pub fn log_lines(&self) -> Vec<String> {
        self.store.data().log.clone()
    }

    /// Hash of everything observable from outside: decisions, log lines and any violation.
    pub fn trace_hash(&self) -> String {
        let s = self.store.data();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (n, v) in &s.choices {
            fnv(&mut h, &n.to_le_bytes());
            fnv(&mut h, &v.to_le_bytes());
        }
        for line in &s.log {
            fnv(&mut h, line.as_bytes());
        }
        if let Some(v) = &s.violation {
            fnv(&mut h, v.as_bytes());
        }
        format!("{h:016x}")
    }

    pub fn snapshot(&mut self) -> Result<Snapshot, WasmError> {
        let memory = self
            .memory()
            .map(|m| m.data(&self.store).to_vec())
            .unwrap_or_default();
        let mut globals = Vec::new();
        let exports: Vec<(String, Extern)> = self
            .instance
            .exports(&mut self.store)
            .map(|e| (e.name().to_string(), e.into_extern()))
            .collect();
        for (name, ext) in exports {
            let Extern::Global(g) = ext else { continue };
            if g.ty(&self.store).mutability() != Mutability::Var {
                continue;
            }
            let value = match g.get(&mut self.store) {
                Val::I32(v) => GlobalValue::I32(v),
                Val::I64(v) => GlobalValue::I64(v),
                Val::F32(v) => GlobalValue::F32(v),
                Val::F64(v) => GlobalValue::F64(v),
                _ => continue,
            };
            globals.push((name, value));
        }
        let fuel = self
            .store
            .get_fuel()
            .map_err(|e| WasmError::Runtime(e.to_string()))?;
        let s = self.store.data();
        Ok(Snapshot {
            memory,
            globals,
            host: HostSnapshot {
                rng: s.rng.clone(),
                choice_rng: s.choice_rng.clone(),
                forced: s.forced.clone(),
                forced_only: s.forced_only,
                fuel,
                cursor: s.cursor,
                choices: s.choices.clone(),
                log: s.log.clone(),
                clock_ms: s.clock_ms,
            },
        })
    }

    pub fn restore(&mut self, snap: &Snapshot) -> Result<(), WasmError> {
        if let Some(mem) = self.memory() {
            let have = mem.data_size(&self.store);
            if snap.memory.len() > have {
                let pages = (snap.memory.len() - have).div_ceil(WASM_PAGE) as u64;
                mem.grow(&mut self.store, pages)
                    .map_err(|e| WasmError::Runtime(e.to_string()))?;
            }
            let data = mem.data_mut(&mut self.store);
            data[..snap.memory.len()].copy_from_slice(&snap.memory);
            data[snap.memory.len()..].fill(0);
        }
        for (name, value) in &snap.globals {
            if let Some(Extern::Global(g)) = self.instance.get_export(&mut self.store, name) {
                restore_global(&mut self.store, g, value)?;
            }
        }
        self.store
            .set_fuel(snap.host.fuel)
            .map_err(|e| WasmError::Runtime(e.to_string()))?;
        let s = self.store.data_mut();
        s.rng = snap.host.rng.clone();
        s.choice_rng = snap.host.choice_rng.clone();
        s.forced = snap.host.forced.clone();
        s.forced_only = snap.host.forced_only;
        s.cursor = snap.host.cursor;
        s.choices = snap.host.choices.clone();
        s.log = snap.host.log.clone();
        s.clock_ms = snap.host.clock_ms;
        s.violation = None;
        Ok(())
    }

    /// Instructions the guest may still execute.
    pub fn fuel_remaining(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }

    /// A new, independent VM in the state captured by `snap`.
    pub fn fork(&self, snap: &Snapshot) -> Result<WasmVm, WasmError> {
        let mut vm = self.compiled.instantiate(0, Vec::new(), self.cfg.clone())?;
        vm.restore(snap)?;
        Ok(vm)
    }
}

fn restore_global(
    store: &mut Store<HostState>,
    g: Global,
    v: &GlobalValue,
) -> Result<(), WasmError> {
    let val = match v {
        GlobalValue::I32(x) => Val::I32(*x),
        GlobalValue::I64(x) => Val::I64(*x),
        GlobalValue::F32(x) => Val::F32(*x),
        GlobalValue::F64(x) => Val::F64(*x),
    };
    g.set(store, val)
        .map_err(|e| WasmError::Runtime(e.to_string()))
}
