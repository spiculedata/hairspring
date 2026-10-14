# Hairspring MVP Spec

Open-source, self-hostable deterministic simulation testing: explore many futures of a system, travel back to any state, replay any failure exactly.

Status: draft, informed by experiments in `experiments/` (std-only Rust, run with `cargo run --release --bin <name>`).

## 1. Goals and non-goals

**Goals (MVP)**
- A developer runs `hairspring test` on a laptop (macOS or Linux) and gets: bug found, minimal repro, one-file replay bundle.
- Same engine runs on a self-hosted server with many workers and a shared store.
- Time travel: pick any node in the exploration tree, inspect state, fork new futures from it.
- Properties as `always` / `sometimes` / `reachable` assertions, plus property-test generators.

**Non-goals (MVP)**
- Deterministic hypervisor or arbitrary-binary determinism.
- Multi-tenant SaaS concerns (billing, org isolation).
- Competing with Hegel/Hypothesis on generators. Interoperate instead.

## 2. What the experiments showed

| # | Experiment | Result | Consequence |
|---|---|---|---|
| 1 | Same code, 100 runs: threads, HashMap order, clock | 5 distinct thread interleavings, 100 distinct HashMap orders, clock always varies | Ordinary programs are non-deterministic by default; every source must be owned by the runtime |
| 2 | Seeded simulation, replay from recorded choices | Same seed gives identical hash; different seed differs; replay matches bit-for-bit | Seed plus choice log is a complete repro. Determinism is testable with a double-run hash check |
| 3 | Random restarts vs snapshot-and-branch (bug needing a split-brain lease overlap) | Median 814 vs 235 steps, mean 1181 vs 272 (300 trials, 300/300 found each) | Branching from novel states beats restarting, even with a naive novelty heuristic |
| 2b | Shrinking by deleting index-based choices | 22 steps to only 19 | Choices must be **stable action IDs**, not indices into the enabled list |
| 5 | Same simulation on Linux x86_64 (AMD EPYC VM) vs macOS arm64 | Identical trace hashes (seed 42: `7a5b5db5097522b3`, seed 43: `4c168db3f75aee30`) and identical exp-3 statistics (median 814 vs 235) | Tier 1 determinism is portable across OS and CPU architecture, so a bundle recorded on a Mac replays on a Linux server |
| 4 | rr inside Docker on Apple Silicon | No `/dev/kvm`; rr aborts: unknown CPU (implementer 0x61) | System-level record/replay is Linux x86 (or supported ARM) only. macOS cannot be a Tier 2 host |

## 3. Platform strategy

| Tier | What is deterministic | macOS | Linux |
|---|---|---|---|
| **1. Simulation runtime** (MVP) | Code written or linked against the Hairspring runtime: virtual clock, seeded RNG, simulated network/disk, controlled scheduler | Full support | Full support |
| **2. Linux system mode** (post-MVP, **determinism unproven**) | Unmodified services in microVMs: snapshot/fork and fault injection give branching, but stock microVMs are NOT deterministic (interrupt/TSC/entropy timing), so replay is statistical until proven otherwise | Not locally. Mac devs submit to a Linux server | Firecracker or Cloud Hypervisor, needs KVM |
| **3. Record/replay of one process** (stretch) | Bit-exact replay of a failing process | No | rr or equivalent, needs PMU access |

Rule: **Tier 1 is the product.** Tiers 2 and 3 plug into the same store, explorer and UI. That keeps the "OSX is a pain" problem confined to optional backends.

### Unverified assumptions (must be tested before relying on Tier 2/3)
- U1: a stock microVM restored from the same snapshot with the same recorded inputs reproduces the same trace. Measure the divergence rate over 100 runs.
- U2: rr works on our target hardware (needs x86 and PMU access, absent on Apple Silicon and most cloud VMs).
- U3: Hermit (or equivalent) still builds and runs a real service deterministically.
- U4: a real Tier 1 target passes `hairspring doctor` without runtime-hostile libraries leaking non-determinism.

## 3b. Determinism support research (2026-10-13)

Source: delegated web research. Findings came through a summarising fetch tool, so "stated" means a page said it, not that raw sources were read. Items marked unknown were not found.

| Approach | macOS arm64 | Linux x86_64 bare metal | Linux cloud VM | Forced exploration (not just replay) |
|---|---|---|---|---|
| In-language sim runtime (ours, madsim, turmoil, shuttle) | Yes (tested) | Yes (tested) | Yes (tested on EPYC VM) | Yes, covers only code written against it |
| WASM (Wasmtime, configured) | Yes (stated) | Yes (stated) | Yes (inferred) | Yes, but needs NaN canonicalisation, virtualised WASI clock/random, no `memory.grow`. No snapshot/fork harness off the shelf |
| rr | No (needs PMU; Apple PMU is private API) | Yes (Intel, some AMD Zen, partial AArch64) | Partial: PMU must be exposed | **No**: record is not deterministic, only replay. Capture tool, not an engine |
| Hermit (Meta) | No (x86-64 Linux only) | Yes: fixes scheduling, time, RNG, CPUID, serialises threads, ~3-6x slowdown | Partial: PMU varies | Yes for run mode. Maintenance status unclear |
| Stock Firecracker/QEMU-KVM snapshot | No (no macOS support; Apple blocks VM snapshot) | Snapshots yes, **determinism no** | Needs nested virt | No |
| QEMU TCG icount record/replay | Partial (TCG only, no HVF) | Yes | Yes (no PMU needed) | Partial: replay only; branching by replaying prefixes. Slowdown unknown |
| Custom deterministic hypervisor (Antithesis; Bedrock OSS, early) | No (Intel VT-x) | Yes, modern Intel only | Effectively no | Yes: one vCPU per VM, virtual time, exact-count VM exits |

Decision rules from this:
1. macOS is a first-class host for **simulation-level** determinism only. No supported system-level path exists (private PMU APIs, entitlement-locked VM snapshots).
2. System-level determinism is Linux x86_64, ideally bare metal with a modern Intel CPU. Cloud VMs are conditional on PMU exposure.
3. Do not promise: any-binary determinism on macOS or arbitrary cloud VMs; stock microVM snapshots as deterministic; rr as an exploration engine; replay across CPU models or kernels.
4. Open decisive experiment: on PMU-capable Intel Linux, run a racy multithreaded binary 100x under `hermit run --strict` and `rr record`, compare trace hashes. Also measure QEMU TCG icount slowdown.
5. Unverified: AWS/Azure/Hetzner/Scaleway/OVH PMU exposure, Bedrock licence, Hermit current maintenance, madsim/Shuttle release dates.

## 3c. Measured: system-level determinism on Intel bare metal (2026-10-14)

Host: Scaleway Elastic Metal, Intel Xeon E5-1410 v2 (Ivy Bridge), kernel 6.8, no hypervisor flag, `/dev/kvm` present. Target: `experiments/racy.c` (4 unsynchronised threads, hashed interleaving). Server deleted afterwards; ~75 min, about EUR 0.27.

| Test | Result |
|---|---|
| Native, 100 runs | Essentially every run different |
| `rr record`, 100 runs | 99 identical, **1 different**: record is *mostly* stable (single-core serialisation) but not guaranteed |
| `rr record --chaos`, 100 runs | Many distinct schedules: diversifies, as documented |
| `rr record` then `rr replay`, 30 pairs | **30/30 exact** |
| Hermit 0.4 (HEAD 2026-10-13) build | Builds, after `libunwind-dev` + `liblzma-dev` and `RUSTFLAGS="-C link-arg=-llzma -C link-arg=-lz"`; Meta committed a cargo-build repair the same day |
| Hermit run (even `/bin/true`) | **Failed**: `failed to initialize ptrace for tracee 3: -22 EINVAL`. Not fixed by relaxing `ptrace_scope` and AppArmor userns restriction. Cause unknown (kernel 6.8? Ivy Bridge? Hermit regression?) |

Conclusions
- rr is confirmed as a **failure-capture and debugging** tool: exact replay, no steering. It is not an exploration engine.
- Hermit's forced-determinism claim is **unverified** here: it builds and is maintained, but did not run on this kernel/CPU. Not disproven. A retry would need a different kernel (e.g. older LTS) or CPU, plus reading its issue tracker.
- Tier 2 therefore has **no working forced-determinism backend yet**. Do not promise it.
- Lesson on method: a script compared two empty strings and reported a pass; always assert non-empty output.

## 3d. Measured: the maintained Hermit fork (2026-10-14) - forced determinism works

Same hardware as 3c (Intel Xeon E5-1410 v2, Ubuntu 24.04, kernel 6.8, bare metal). Software: `rrnewton/hermit` (the fork its README calls "maintained"), `hermit 0.4.1 (2026-10-14)`, built with `cargo build --release --manifest-path hermit-cli/Cargo.toml` (the full `--workspace` build fails on the optional `reverie-sabre`/`reverie-dbt` backends; not needed). Same box, so the failure in 3c is attributable to the Meta repo's build (`facebookexperimental/hermit`, older Reverie line), not to the hardware or kernel.

| Test (racy 4-thread binary) | Result |
|---|---|
| `hermit run` x100 | **100/100 identical** (`33c6565718b72143 idx=80000`); native was ~100 distinct, rr 99/100 |
| `--sched-seed` only, 20 seeds | All identical: default heuristic ignores the seed for this program |
| `--sched-heuristic=random --sched-seed=N`, 20 seeds | **8 distinct interleavings**: the seed explores |
| Same random seed run 5x, 20 seeds | **20/20 fully reproducible** |
| `--chaos --sched-seed=N` twice | SAME for 3/3 seeds, differing across seeds |

Meaning: for an unmodified multithreaded binary, Hermit gives *seeded, reproducible schedule exploration*, which is the forced-determinism property rr lacks and the building block for a Linux system tier. It also saves each run's inputs as a recording (`hermit run --from <dir>`).

Caveats (do not oversell)
- One CPU-bound, syscall-free test program on one old Intel CPU. No network, disk, processes, or real service tested.
- Preemption uses PMU counters; the README says it runs without them with fewer preemption points. **Untested on a VM.**
- The fork is a small third-party repo (2 stars) with heavy bot-style activity; code unreviewed. Treat as untrusted until audited.
- Hermit is x86-64 Linux only: no macOS, no arm64.
- Default build needs `libunwind-dev` and `liblzma-dev`; full workspace build needs more.

Revised decision: Tier 2 = Hermit-backed system mode on x86-64 Linux, labelled experimental; Tier 1 (+WASM) remains the cross-platform product.

## 3e. Later results (summary)

Measured after the sections above; details in [docs/determinism.md](docs/determinism.md).
- The Hermit "first run differs" effect was **host filesystem state**: Hermit virtualises timestamps, so Python's
  `.pyc` caches look stale after any native Python run and are recompiled and rewritten. Freezing them
  (`PYTHONDONTWRITEBYTECODE=1`, empty `PYTHONPYCACHEPREFIX`) gave 50/50 identical runs.
- The Hairspring Hermit backend, run on bare metal, found and reproduced 3/3 a real threading bug in C and a
  duplicate-delivery bug in an unmodified Python Dapr consumer; the fixed variants passed every seed.
- WASM trace hashes are identical on macOS arm64 and Linux x86-64.

## 4. Architecture

```
 SDK (Rust first, then Go)            hairspring CLI / server
 ┌──────────────────────┐            ┌───────────────────────────────┐
 │ assertions            │  events    │ explorer  (picks node, branches)│
 │ choice source         │──────────▶│ store     (node DAG + blobs)     │
 │ sim runtime           │            │ api/ui    (timeline, replay)    │
 └──────────────────────┘            └───────────────────────────────┘
        worker process  ◀── jobs ──  scheduler (local: in-proc, server: queue)
```

Components
1. **SDK**: `Sim` trait (`enabled() -> Vec<ActionId>`, `apply(ActionId)`, `snapshot() -> Bytes`, `restore(Bytes)`, `signature() -> u64`) plus assertion macros.
2. **Sim runtime**: virtual time, seeded RNG, simulated network with drop/dup/reorder/partition, simulated disk with torn writes, scheduler hook for task interleaving. Everything non-deterministic is behind a trait the runtime owns.
3. **Explorer**: loop of pick snapshot, restore, run N steps with random or guided choices, record novel signatures, stop on violation. MVP heuristic is the least-visited signature (as in exp 3). Pluggable.
4. **Store**: DAG of nodes, content-addressed snapshot blobs, event log.
5. **CLI/server**: one binary, two modes (see section 7).
6. **UI**: timeline scrubber over the DAG, state viewer, replay button, diff between two nodes.

## 5. Data model

```
Run     { id, target, build_hash, seed, started_at, config }
Node    { id, run_id, parent_id, edge: ActionId, depth, sim_time,
          signature: u64, snapshot_blob: Hash | null,   // sparse: snapshot every k nodes, else replay from parent
          verdict: ok | violated | sometimes_hit(prop) }
Action  { id: stable u64 hash of (kind, subject, args) }   // NOT an index
Event   { node_id, kind: log | assertion | fault, payload }
Bundle  { run seed, build_hash, root snapshot, [ActionId...] }   // the one-file repro
```

Invariants
- A node is reproducible from `(root, path of ActionIds)` alone. Snapshots are an optimisation.
- Blobs are chunked and content-addressed so sibling snapshots share storage.
- Shrinking deletes ActionIds and re-validates that each remaining action is still enabled (skips otherwise).

## 6. Determinism contract

1. No ambient time, randomness, thread spawn, network, filesystem or env reads in simulated code; all go through the runtime.
2. Iteration order must be defined (no `HashMap` iteration without a fixed hasher).
3. **`hairspring doctor`** runs every target twice with the same seed and compares a rolling hash of the trace. It fails on first divergence and prints the step. This is exp 2 as a product feature, and it runs in CI.
4. Build hash is recorded; a bundle only replays on the same build hash (or warns).

## 7. Deployment

| | Local | Self-hosted |
|---|---|---|
| Binary | `hairspring` single binary | Same binary: `hairspring server`, `hairspring worker` |
| Metadata | SQLite | Postgres |
| Blobs | `.hairspring/` dir | S3-compatible (MinIO) |
| Scheduling | In-process thread pool | Queue (Postgres `SKIP LOCKED` to start) |
| Auth | None (localhost) | Token / OIDC later |
| Platform | macOS, Linux | Linux workers (Tier 2 later) |

Mac developers get full Tier 1 locally. Tier 2 jobs submit from the same CLI to a Linux server (`hairspring test --remote`).

## 8. Milestones

| M | Deliverable | Exit criterion |
|---|---|---|
| M0 | `experiments/` (done) | Determinism, replay and branching demonstrated |
| M1 | `hairspring-core`: Sim trait, stable ActionIds, bundle format, shrinker, `doctor` | Exp 3 target re-implemented on the trait; shrinker returns a 1-minimal trace (no single step removable; not necessarily globally minimal, e.g. 8 vs 11 steps depending on the failure found); doctor catches an injected `HashMap` leak. **Done: 16 tests, `examples/lock_demo.rs`** |
| M2 | Store plus explorer with SQLite and chunked snapshots | 10k-node run; memory flat; any node restorable |
| M3 | Simulated network/disk/clock library and a real target (small Raft or Dapr-pubsub consumer) | Finds a planted bug in under 1 min on a laptop |
| M4 | Server and worker mode, Postgres, MinIO, web timeline UI | Multi-worker run visible live; click-to-replay works |
| M5 | Tier 2 spike: Firecracker snapshot/fork of a docker-compose target on Linux | One service, one network fault, branch from a snapshot |

## 9. Open questions

1. Snapshot mechanism for Tier 1: serialise state (serde/bincode, portable, slower) versus fork-the-process (fast on Linux, awkward on macOS)? Leaning serialise for the MVP.
2. Rust only for the MVP, or Go in parallel (`synctest` gives scheduler control cheaply)?
3. Interop with Hegel: adopt its protocol for generators, or expose an adapter?
4. Licence: Apache-2.0 for core; decide on server and UI.
5. Which real target proves M3? (A real Dapr-based service is a natural candidate.)
