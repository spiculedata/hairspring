# What we measured about determinism

Everything here was measured on real machines during development; the numbers come from the programs in
[`experiments/`](../experiments) and from the integration tests. Where something is unverified, it says so.

## The levels

| Level | Meaning | Result |
|---|---|---|
| 1. Simulation | Code written against our runtime gives the same result for the same seed | Verified on macOS arm64 and Linux x86-64: identical trace hashes and identical exploration statistics |
| 2. WASM | A guest program using the host-provided `choose/rand/fail` imports | Verified: the golden trace hash recorded on macOS arm64 is identical on Linux x86-64 |
| 3. System | An unmodified Linux program, scheduled deterministically | Verified on Intel bare metal with Hermit (below). Not possible on macOS |

## System-level: record/replay and forced determinism (Linux)

Host: Scaleway Elastic Metal, Intel Xeon E5-1410 v2 (Ivy Bridge), Ubuntu 24.04, kernel 6.8.

| Test (a 4-thread racy C program) | Result |
|---|---|
| Native, 100 runs | essentially every run different |
| `rr record`, 100 runs | 99 identical, 1 different: record is mostly stable but not guaranteed |
| `rr record` then `rr replay`, 30 pairs | 30/30 exact |
| Hermit (maintained fork), 100 runs | **100/100 identical** |
| Hermit, 20 seeds with `--sched-heuristic=random` | 8 distinct interleavings; each seed reproduced 5/5 |

Conclusions: `rr` gives exact replay of one captured failure but cannot steer or branch, so it is a debugger,
not an exploration engine. Hermit's seeded scheduler is both: seeds explore, and a failing seed replays exactly.

Real programs under Hermit (via the Hairspring backend):

- A check-then-act threading race in a C program was found at sched-seed 2, reproduced 3/3, and the fixed
  program passed all 60 seeds.
- A simulated Dapr sidecar with an unmodified Python consumer (`examples/dapr-orders`): the buggy consumer
  failed (`order 1 was applied 2 times`) at sched-seed 1 and reproduced 3/3; the fixed consumer passed 12/12 seeds.

### Which Hermit

Two repositories exist. The original (`facebookexperimental/hermit`) failed to start on our machine
(`failed to initialize ptrace for tracee`, even for `/bin/true`). The maintained fork (`rrnewton/hermit`)
worked on the same machine and kernel. Build only the CLI (`cargo build --release --manifest-path hermit-cli/Cargo.toml`);
the full workspace build also compiles optional backends that need extra C headers. It needs `libunwind-dev`
and `liblzma-dev`, and, for the original repo, a link flag. It is a small, fast-moving project: review it first.

### Hidden inputs: the host is part of the program

Hermit's own documentation says the host filesystem is visible to the guest and results depend on the
*contents* of those files. We found this the hard way. The first Hermit run after any native Python run
differed, in virtual time (`busy_ms` 65 vs 70) and by one memory mapping, then stayed stable. Reproduced on demand:

| Condition | Result |
|---|---|
| Default Python under Hermit, after a native Python run | the first run differs, then returns to normal |
| With `PYTHONDONTWRITEBYTECODE=1` and an empty `PYTHONPYCACHEPREFIX` | identical after native runs; 50/50 identical |

Cause (verified from Hermit's debug log): Hermit virtualises file timestamps, so Python considers its cached
`.pyc` files stale after a native run rewrote them with real timestamps, recompiles, and rewrites them. The
Hermit backend therefore starts guests from a minimal pinned environment and freezes bytecode caches. Idle
time, CPU load and drop-caches did not matter (110/110 runs identical across gaps up to 90 s).

### Not measured / not possible

- **macOS**: there is no supported path to system-level determinism. Apple Silicon exposes performance
  counters only through private APIs, Docker Desktop has no `/dev/kvm`, and rr does not recognise the CPU.
  Hermit is x86-64 Linux only. macOS gets levels 1 and 2.
- **Cloud VMs**: Hermit prefers hardware performance counters (its README says it can run without them with
  fewer preemption points). We tested only bare metal. We did not test an ordinary cloud VM.
- Stock microVM snapshots (Firecracker, etc.) are not deterministic after restore (timers, interrupts,
  entropy); we did not build on them. A deterministic hypervisor (as Antithesis built) is out of scope.
- QEMU TCG record/replay was not measured.

## Honest statistics

- Branching beat restart-from-scratch by about 3.5x (median 235 vs 814 steps, mean 272 vs 1181, 300 trials) on
  a toy lock-lease target. That is one target; your system will differ.
- The in-process doctor detects ordering leaks probabilistically: with two deliverable messages a `HashMap`
  order leak shows up roughly half the time per opportunity, so check many seeds.
