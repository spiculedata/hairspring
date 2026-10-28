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

A bundle also does not record the program's command line or working directory: replay runs the command and directory from the current configuration. A program whose behaviour depends on the text of its own paths can follow a different schedule when the same files run from a differently named directory, and the replay then reads as a mismatch even though the recorded environment matches. This has been seen with a JVM, where the install path, the class path and the working directory are part of what the program reads. Replay from the same absolute paths as the recording.

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
line beyond `hermit_args` (the program's own command line and working directory are covered above), the `--tmp` pre-seed, and the Hairspring commit. The environment is compared by
`hairspring confirm` (see [Confirming a failure](#confirming-a-failure)), but not by the search's own three reruns or on
bundle import. Each new field needs its name added to the fixed list in `hairspring-core`.

Upgrade order: a server that predates v3 rejects a v3 bundle (it does not recognise the header). Upgrade the server
before the workers that send it bundles.

Rollback: a server rolled back to a version that predates v3 cannot replay the v3 bundles already stored, because it
rejects the header. Older readers ignore the extra `param.` lines, so the only change needed is the first line,
`hairspring-bundle v3` to `hairspring-bundle v2`, edited by hand. Rolling back without doing that is not supported.

The 409 body from `POST /api/replay` is `{"error": "<short text>", "code": "environment_mismatch"}`; the MCP tool
error has the same `code` in `structuredContent`.

### Confirming a failure

The search confirms a failing seed by running it again three times and comparing a behaviour hash. That is weaker than
what it takes to trust a finding, so `hairspring confirm <run-id | failure-id | bundle>` runs explicit, recorded checks.
For a run or failure id every check is stored; for a bundle file, `-` or a bundle hash it is only printed.

| Check | What it does | Passes when |
|---|---|---|
| `same_seed_rerun` | the seed run again `--reruns` times (default 3, at most 10), each with a fresh private directory, the pinned environment and stdin from `/dev/null` | every rerun fails the same way: the same kind of failure, the same property, the same behaviour hash and the same schedule fingerprint as the recording |
| `bundle_replay` | the bundle replayed, with the environment check | the failure repeats; a miss is a failure of the check only on an environment that matches the recording |
| `fixed_control` (`--control <target>`) | the same seed, scheduler flags, Hermit flags and guest environment on a second target that runs the fixed build | the fixed build is clean at the seed. A control whose flags differ is not a control and nothing is run. The working directory, the `--tmp` pre-seed and the events transport are not compared. It says "the fixed build does not fail at this seed", never "the same interleaving": a different build is a different schedule |
| `strict_verify` (`--strict-verify`) | `hermit run --verify --verify-allow=failure --verify-strict --verify-json=F` (two runs) | the result file says `verified`, `bitwise_parity` and a `matched` verdict |

Each check ends as `pass`, `fail`, `inconclusive` or `skipped`, with counts of the runs that passed, failed or could
not be judged. A rerun that cannot be judged (a timeout, a Hermit fault, an events fault) is counted apart and is never a
pass or a miss. A check that did not run because the caller cancelled, or the wall budget (`--wall-secs`, at most 600)
was used up, is stored as `skipped` and says why. The budget and the cancel reach every run, the replay and each skid
retry included: a run is given only what is left of the budget (at least one second), and a rerun, replay or control the budget cut short
is `inconclusive`, never a miss, a hang or (where a hang is the bug) a violation.

**Cancelling.** A cancelled confirmation is recorded as `skipped` only for a caller that uses the progress callback (the
server and the embedded engine). `hairspring confirm` on the command line has no signal handling: Ctrl-C stops
hairspring, records nothing for that confirmation, and may leave Hermit child processes running, exactly as it does for
`hairspring test`.

**The environment.** The host is compared with the bundle's recorded environment once, before anything runs. If a
loader field differs and `--allow-environment-change` is not given, nothing runs: the rerun and replay checks are stored as
`inconclusive: environment differs`. If another field differs, the checks run, and a miss is `inconclusive: environment
differs`, never a failure and never "nondeterministic". `hairspring confirm` exits 3 for it.

**Skid overshoot.** With `skid_margin` set, Hermit panics when a preemption lands late. That run is infrastructure, not a
verdict. A confirmation retries it once at the same flags, then once at double the margin (up to Hermit's limit). A run at
a higher margin is a different schedule: it cannot count toward the reruns (the check is then `inconclusive`, with a
`caveat` and `skid_margin_doubled: true`), but it can count for the control, with the same caveat recorded.

**No logging.** A target whose `hermit_args` carry `--log`, `--log=...`, `-v...` or `--verbose`, or whose Hermit
environment carries `HERMIT_LOG`, cannot be confirmed (a clear error, nothing run): enabling logging changed a guest's
path.

**The finding.** Its state is derived from the stored checks alone, and from ONE confirmation: the newest that
finished (nothing skipped), or, when none finished, the newest with its skipped checks counting for nothing. Checks
taken at different times are never mixed, so an old strict verify does not lift a newer confirmation to `verified`, a
newer cancelled confirmation does not revert a finished one, and "under the recorded environment" needs the reruns
and the replay of that same confirmation (the same identity, unchanged).

| State | When |
|---|---|
| `candidate` | no check has repeated the failure (also while the only results are inconclusive or skipped) |
| `rerun_repeated` | the reruns repeat it; no replay of the bundle has yet. This is not "reproduced", and the claim wording never says so |
| `confirmed` | and the replay does too |
| `verified` | and the strict verify agrees |
| `not_reproducible` | a rerun check or the replay failed on an unchanged environment |
| `control_failed` | reruns repeat it but the fixed build still fails at the seed |

Only `confirmed` and `verified` reach the claim wording: a generated sentence says "Reproduced (confirmed|verified)" for
them, names the environment ("under the recorded environment", or "environment not pinned by a recorded identity" when
the bundle recorded none or the host differs), and uses no other state. For a claim the state is derived from the
confirmations of trusted runners only: a stored check from a runner that is not trusted never opens a claim, and cannot
lift or complete what a trusted runner found. The word "found" is never generated.

Stored checks are served by `GET /api/failures/{id}/checks` and the `get_failure_checks` tool; the command and notes
a runner reported are under `guest`, and the runner's own id is stored but never served. The answer names the
`confirmation_id` the finding is derived from. The tool's answer is cut to fit one reply (the oldest checks first,
never the ones the finding comes from, with `truncated` and `checks_omitted` saying so); the REST endpoint serves all of
them. `hairspring show` summarises the findings of a run. `hairspring confirm --json` prints one JSON document: an
object for a failure id or a bundle, an array with an entry per failure for a run id. The lines `hairspring confirm`
prints are those of the confirmation the finding comes from, and it says when the one just stored did not finish.

**What a stored `command` is.** A record of the invocation, not a line to paste and run. It names the program by file
name, leaves out the per-run plumbing (summary file, events directory) and shows the strict verify's result file as
`verify.json`. A real run clears the guest's environment and rebuilds it, sets `HAIRSPRING_SEED`, the backend's
`extra_env` and the events variables outside the command line, and runs in a private directory, so pasting it into a
shell does not reproduce the check. It includes the target's full command and `hermit_args`, and every reader of the
check (REST, MCP, the web UI, `hairspring show`) sees it: **do not put credentials in a target's arguments**, they are
stored and served as written. Secret-looking values are not redacted, because a partial redaction would give false
comfort.

**Storage and retries.** A confirmation is stored under an id derived from its content (the failure, the runner and
every check), so a retry of the same report stores nothing twice; a new confirmation differs at least in its times.
Checks list by time and then by the order they were written. A failure holds at most 200 checks: a confirmation that
would pass that drops the oldest whole confirmations to make room, so the newest checks are kept; untrusted confirmations go before trusted ones. A report the cap already dropped can be sent again and is stored as the newest. (No remote reporting path exists yet, so neither is reachable today.) Text is cut by
bytes (after JSON escaping) to fit the 16 KiB column, so a long or wide-character value is shortened and never refuses a
confirmation.

**Known limitation: simulation targets.** A failing seed from a simulation (`sim`) target, in a sweep or a search, cannot be confirmed: `hairspring confirm` stops with ``error: no backend available for target kind ``confirm` is not supported by the sim backend` `` (the stray backticks are in the message) and nothing is stored. Only `process` targets can be confirmed.

**Sweep failures: one row for each property's first failing seed.** A sweep stores a failure row (with its trace and
replayable bundle) for the lowest failing seed and for every later seed at which a property failed for the first time.
Two properties that first fail at the same seed share one row, and a property that fails again at a later seed gets no
further row, so a sweep in which only one property fails has one row, as before. The claim gate needs every
first-failure seed of the run confirmed, and each of these rows can be confirmed: `hairspring confirm <run id>`
confirms every failure of the run in turn and stores the checks for each (a sweep does not confirm its rows by itself
when it ends; each row's message says how many of the sweep's own reruns matched). A sweep stores at most 16 rows; a
property whose first failing seed comes after that has no row, and the run's claim stays withheld. A failure that
does not name a property (a non-zero exit or a `HAIRSPRING_VIOLATION:` line) only ever has the row of the lowest failing
seed. Rows are read in the order they were written, which is seed order. No stored data changes: the failures table
already held any number of rows for a run, and a sweep stored before this change has the rows it had.

**Other limits.** SQLite allows one writer at a time. A write takes the write lock when its transaction begins and waits
up to 5 seconds for another process to finish; if the file is still locked after that it fails with
`storage error: database is locked` and should be retried. The local runner label comes from the process, so it is not the same worker identity in the CLI and in the embedded
engine (or across restarts): a confirmation is attributed to the process that ran it, and the two are not treated as one
runner.

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
