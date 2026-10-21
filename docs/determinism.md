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

### Recorded environment

The measurements above mean a recorded schedule depends on more than the seed and the Hermit flags. Two cases
seen in practice: a package install regenerated `/etc/ld.so.cache`, and the same seed then ran a different number
of loader branches before opening libc, so old recordings stopped replaying; and with a host-derived environment the
launch environment changed the schedule. Without a record, a bundle that fails to reproduce looks like
nondeterminism.

A Hermit bundle therefore records these fields (as `param.<name>` lines, which makes it a v3 bundle):

| Field | What it is | Safe to publish |
|---|---|---|
| `host.kernel_release` | `uname -r` | yes |
| `host.cpu_model` | first `model name` in `/proc/cpuinfo` | yes |
| `host.cpu_flags.sha256` | hash of the sorted CPU feature list | yes |
| `host.loader.ld_so_cache.sha256`, `host.loader.libc.sha256`, `host.loader.ld_so.sha256` | hashes of the loader cache, libc and the dynamic loader (glibc and musl, x86-64 and aarch64 locations) | yes |
| `host.loader.ld_so_preload.sha256` | hash of `/etc/ld.so.preload`, or `absent` | yes |
| `host.container` | `docker`, `podman` or `none`, from the usual marker files | yes |
| `hermit.path.sha256`, `hermit.version` | hash of the Hermit executable and the first line of `hermit --version` (left out if it does not finish within a second; asked again after an upgrade in place, because the answer is kept per executable hash) | yes |
| `env.base_env` | the `--base-env` mode in the target's `hermit_args`: `minimal`, `host` or `empty`; `default` when the flag is absent (Hermit's own default applies); `unknown` when the value is not one of those | yes |
| `env.guest.names` | the sorted names of the variables the guest and the Hermit process are given; replaced by `env.guest.names.sha256` (a hash of the same list) when the list is too long to record | names can reveal what a deployment uses; check them before publishing a bundle |
| `env.guest.allowlisted.sha256` | hash of `NAME=value` lines for an allowlist of non-secret variables only (`PATH`, `LANG`, `LANGUAGE`, `TZ`, `LC_*`, `PYTHONDONTWRITEBYTECODE`, `PYTHONHASHSEED`) | yes. Values of every other variable are not recorded or hashed, since a hash of a short secret can be guessed offline |
| `target.exe.sha256` | hash of the program the command starts, only if it is a regular file (not of its arguments or input files) | yes |

Only these names exist. A bundle becomes v3 only when it carries one of them; a `host.*` or `env.*` param with any
other name is ignored on read, so an unknown or hostile param can neither change the header nor be compared.

No path, host name or user name is stored: files are named by role, not by where they were found. A source the
host does not offer is left out rather than invented. On replay such a field is reported as `now unavailable`, and
that counts as changed: a recorded loader hash whose file is now missing refuses the replay, and a recorded
`hermit.version` that does not answer within the probe's second (or a missing `/etc/ld.so.cache`) makes a miss
exit `3`. A `--version` probe that gets no answer is not repeated for the lifetime of the backend. Recorded values are
printable ASCII only, and the `.sha256` fields are lowercase hex (or `absent` for the preload file); a recorded value
that is not is malformed, is named in a warning, is not compared, and also counts as changed. The values are read when the run starts, before any trial or confirmation rerun, and they
are the same fields for every failure the run reports.

On replay, the recorded fields are compared with the current host. Each recorded field is checked on its own: a
malformed one is named in a warning and skipped, and the others are still compared.

- `warning: environment differs: <field> (recorded X, now Y), ...` lists the fields that changed (hashes are
  shortened to 12 characters; after eight fields the rest are counted as `and N more`). The replay still runs. If it
  does not reproduce the failure, the trace says that the environment differs, so the miss is not evidence the bug is
  fixed.
- If a `host.loader.*` hash changed, replay is refused, because the schedule itself is unreliable. The refusal is its
  own error kind, not an internal error: HTTP 409 on the REST API, and a tool error that starts with `environment
  mismatch` on MCP. `hairspring replay --allow-environment-change`, `"allow_environment_change": true` on
  `POST /api/replay` and the `allow_environment_change` argument of the MCP `replay_bundle` tool replay anyway and
  still return the warnings. The default is always strict.
- A bundle recorded before this existed prints `warning: no environment recorded in this bundle` and replays as
  before.
- `warning: hermit.path.sha256 was not recorded: a different Hermit build would not be noticed` appears when the
  bundle has an environment but no Hermit hash, and `... was not recorded and cannot be read now: ...` when the hash
  cannot be read on this host either. Nothing is compared in either case, so the line says so instead of staying
  silent. A hash that was recorded but cannot be read now shows as `hermit.path.sha256 (recorded X, now unavailable)`
  in the `environment differs` line.
- `warning: malformed recorded environment fields, not compared: <fields>` names recorded values that are not valid;
  they count as changed.

`hairspring replay` exit codes: `0` not reproduced on the environment it was recorded in (or one that cannot be
compared), `1` reproduced, `2` error (including a refusal), `3` not reproduced on a changed environment. `3` is
inconclusive: it does not show the bug is fixed. The API returns the same fact as `environment_changed` in the replay
result.

Not recorded yet: the Hermit commit, Hermit's global seed, hashes of declared input files, the full Hermit command
line beyond `hermit_args`, the `--tmp` pre-seed, and the Hairspring commit. The environment is also not compared on
confirmation reruns or on bundle import. Each new field needs its name added to the fixed list in
`hairspring-core`.

Upgrade order: a server that predates v3 rejects a v3 bundle (it does not recognise the header). Upgrade the server
before the workers that send it bundles.

Rollback: a server rolled back to a version that predates v3 cannot replay the v3 bundles already stored, because it
rejects the header. Older readers ignore the extra `param.` lines, so the only change needed is the first line,
`hairspring-bundle v3` to `hairspring-bundle v2`, edited by hand. Rolling back without doing that is not supported.

The 409 body from `POST /api/replay` is `{"error": "<short text>", "code": "environment_mismatch"}`; the MCP tool
error has the same `code` in `structuredContent`.

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
