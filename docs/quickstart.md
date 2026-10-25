# Quick start: the `process` backend

This page takes a developer from nothing to running an unmodified Linux program under Hermit with Hairspring, and
replaying what it finds. It covers the `process` backend only. The `sim` and `wasm` backends need none of this: see the
[README](../README.md).

Everything here describes what the scripts and commands do. It makes no claim about how often a bug is found or how
fast a run is: both depend on your machine and your program. What was measured, and where, is in
[determinism.md](determinism.md).

## What you need

| Need | Why | How to check |
|---|---|---|
| Linux on x86-64, bare metal or a VM that passes the CPU's performance counters through | Hermit counts hardware events to schedule threads deterministically. Apple silicon and macOS cannot run it. Ordinary cloud VMs and containers were not tested here and often do not expose the counters | `scripts/install-hermit.sh check` |
| `kernel.perf_event_paranoid` low enough for your user (or run as root) | Opening a hardware counter | same |
| `kernel.yama.ptrace_scope` of 0 or 1 (or run as root; 2 needs root, 3 disables tracing) | Hermit traces the program | same |
| Linux 6.9 or newer (recommended) | Older kernels take a slower ptrace path. Schedules are not affected, timeouts may be | same (reported as a note) |
| To build Hermit: `git`, a C compiler, `pkg-config`, `libunwind` and `liblzma` development files, and [rustup](https://rustup.rs) | Hermit is built from source from a pinned fork and commit; there is no binary download | `scripts/install-hermit.sh install --dry-run` |
| To build the `hairspring` CLI from source: a stable Rust toolchain. Or download a release binary | | |

The scripts never use `sudo`, never change a kernel setting and never install system packages. When a setting is in
the way they name it and leave the decision to you; lowering a security setting is a trade-off for the owner of the
machine.

The measurements behind this project ran Hermit as root. Running it as an unprivileged user was not tested.

## Install

From a checkout of this repository:

```bash
# 1. Can this machine run Hermit at all? Read-only; exit 0 = yes, 1 = no, with the reasons listed.
scripts/install-hermit.sh check

# 2. Build the pinned Hermit from source and install it (default prefix ~/.local/share/hairspring/hermit).
#    --install-toolchain lets the script run 'rustup toolchain install nightly-2026-07-12' for you; without it a missing
#    toolchain is an error that prints the command.
scripts/install-hermit.sh install --install-toolchain

# 3. Build the hairspring CLI from this checkout (cargo build --release --locked) and install it to ~/.local/bin.
scripts/install.sh

# Or, instead of 3 and once a release has been published, download its binary. The sha256 is checked against the
# release's SHA256SUMS before anything is installed.
scripts/install.sh --version v0.1.0

# 4. Tell Hairspring which Hermit to use, and check the whole picture in one place.
export HAIRSPRING_HERMIT="$HOME/.local/share/hairspring/hermit/bin/hermit"
hairspring doctor --process-backend
```

What the Hermit installer pins and verifies:

- the fork (`github.com/spiculedata/hermit`, a fork of `rrnewton/hermit`) at one commit, checked after checkout;
- the replay patch (`examples/glibc-condlock/experiment/patches/hermit-25781c5-replay-backport.diff`) by sha256 before
  anything is cloned, and that it applies;
- the Rust toolchain `nightly-2026-07-12`, with `cargo build --release --locked`;
- the built binary: `hermit --version` runs, `--help` lists `--log`, and `hermit run --help` lists the flags the
  Hairspring backend passes.

It prints the sha256 of the binary it installed and records it, with the pins, in `hermit.provenance` next to it. A
rebuild is not expected to give the same sha256, so a recording made with one Hermit build is tied to that build:
Hairspring's bundles record the Hermit executable's hash for this reason.

`hairspring doctor --process-backend` reports, in one place, whether Hermit was found (and its version and sha256),
whether a hardware counter can be opened, the ptrace restriction, the kernel, and the C library and loader files. Its
last line is either `result: ok: ...` or `result: CANNOT RUN the process backend on this machine:` followed by the
reasons, and the exit code is 0 or 4 respectively (4 is distinct from 1, which means a property was violated). `ok`
means the prerequisites are met; it does not mean a program has been run under Hermit here.

Hermit's replay behaviour depends on the machine it runs on (CPU, kernel, loader). If you move from one machine to
another, expect to record again.

## Run a C example

The language fixtures in [`examples/language-fixtures`](../examples/language-fixtures/README.md) are small programs
with a planted lost-update race, instrumented with the [C SDK](c-sdk.md). `at-c` takes its mode as the only argument:
`buggy` (the race), `fixed` (a mutex) or `wide` (a larger window).

```bash
make -C examples/language-fixtures/c CC=gcc          # builds build/gccO2/at-c
mkdir -p /tmp/hs-demo && cd /tmp/hs-demo
hairspring init                                       # writes a starter hairspring.toml
```

Add the target to `hairspring.toml` (use the absolute path of the program you built):

```toml
[target.at-c-buggy]
kind = "process"
command = ["/path/to/hairspring/examples/language-fixtures/c/build/gccO2/at-c", "buggy"]
trial_timeout_secs = 120

[target.at-c-buggy.budget]
max_trials = 20
```

Then:

```bash
hairspring test at-c-buggy --seed 1
```

Each trial runs the program under Hermit with a different scheduler seed. The program always exits 0; the verdict comes
from the property it reports through the SDK (`c.counter_exact`, which says the final counter equals 4,000). A trial
that does not finish within `trial_timeout_secs` is reported as inconclusive, never as a pass.

## Read the output

- The command exits 0 when every target passed, 1 when a property was violated, 2 on an error. With a violation it
  prints the target, the scheduler seed, and where the bundle was written (`.hairspring/bundles/`).
- `--format json`, `junit` or `sarif` write the same result for tools.
- `hairspring results` lists stored runs; `hairspring show <run id>` shows one, including the properties it checked.
- A small budget may or may not hit the race. Whether it does depends on the machine and the budget, so a pass with a
  small budget says only that those seeds did not fail. Raise `max_trials`, or run the `fixed` mode for comparison
  (the same program with a mutex).

## Replay a bundle

```bash
hairspring replay .hairspring/bundles/<bundle>.hairspring
```

Replay re-runs the recorded seed under Hermit and compares the host with the environment written into the bundle (kernel,
CPU flags, loader files, Hermit executable hash, the target program's hash). The exit code says what happened:

| Exit | Meaning |
|---|---|
| 0 | not reproduced, on the environment it was recorded in |
| 1 | reproduced: the property is still violated |
| 2 | error, including a replay refused because the dynamic loader changed (`--allow-environment-change` overrides the refusal and still prints the differences) |
| 3 | not reproduced on a changed environment: inconclusive, not evidence the bug is fixed |

See [Recorded environment](determinism.md#recorded-environment) for the fields and what they mean.

## What is supported

| Language | SDK | Fixture | SDK / fixture exists | Run under Hermit (evidence) |
|---|---|---|---|---|
| C | [`sdks/c`](../sdks/c), [c-sdk.md](c-sdk.md) | `examples/language-fixtures/c` | yes | yes, see the [glibc showcase](showcase-glibc-lost-wakeup.md) |
| C++ | the C SDK (clang++ is the supported compiler; see c-sdk.md for g++) | `examples/language-fixtures/cpp` | yes | not yet confirmed |
| Go | [`sdks/go`](../sdks/go) | `examples/language-fixtures/go`, `examples/go-sql-cancel` | yes | yes, see the [Go showcase](showcase-go-sql-cancel-race.md) |
| Rust | [`sdks/rust`](../sdks/rust), [rust-sdk.md](rust-sdk.md) | `sdks/rust/fixtures` | yes | not yet run under Hermit |
| Python | [`sdks/python`](../sdks/python), [python-sdk.md](python-sdk.md) | `examples/language-fixtures/python` | yes | single-threaded CPython only; **not supported yet** as a `process` target |
| Java | in progress | in progress | a probe program only; SDK and fixture in progress | not yet run under Hermit |
| .NET | [`sdks/dotnet`](../sdks/dotnet), [dotnet-sdk.md](dotnet-sdk.md) | `examples/language-fixtures/dotnet` | yes | not yet run under Hermit; **.NET is not supported on this build** |

"SDK / fixture exists" means the code and its tests are in this repository. Only the last column says whether a
program in that language has been run under Hermit.

A program does not need an SDK to be run: the backend runs any Linux command and treats a non-zero exit as a failure.
An SDK adds named properties (`always`, `sometimes`, `reachable`) that are tracked across trials.

## Platform requirements and known limits

- **Linux x86-64 only** for the `process` backend, on hardware that exposes performance counters. macOS and Apple
  silicon cannot run it. Only bare metal was tested; ordinary cloud VMs and containers were not, and often do not
  expose counters. The CLI itself, and the `sim` and `wasm` backends, run on macOS and Linux.
- **Hermit is a third-party project** that Hairspring does not control, and a young one. The installer builds a pinned
  fork at a pinned commit with a small backport patch. Read what it builds before running it on anything important.
- **The guest sees the host.** Files, environment and timestamps are inputs. The backend starts the program from a
  minimal pinned environment and freezes Python's bytecode caches, but other mutable host state can still change a
  schedule. A package upgrade that changes the loader can stop an old bundle replaying; replay says so.
- **Runs are slower than native**, by an amount that depends on the program and the kernel. Set `trial_timeout_secs`
  accordingly.
- **Privileges.** The recorded measurements ran Hermit as root. Unprivileged use was not tested.
- **Inconclusive is not pass.** A trial that times out, or a replay on a changed environment that does not fail, does
  not show a bug is absent.
- **Experimental.** APIs and the bundle format will change. The limits in the README apply.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `result: CANNOT RUN ... performance counters (PMU)` | A VM or container that does not expose counters. Use bare metal or a VM that passes the PMU through |
| `perf_event_paranoid is N` | Your kernel denies counters to unprivileged users. Run as root, or change the setting if your policy allows. The scripts never do |
| `ptrace_scope is N` | Yama restricts tracing. 2 needs root; 3 disables it until reboot |
| `Hermit was not found` | `HAIRSPRING_HERMIT` is unset and `hermit` is not on `PATH`. Export the path the installer printed |
| `Inherited descriptor cleanup failed ... close_range` | The environment (an emulated or restricted container) does not implement `close_range`. Run on a native Linux host |
| `environment mismatch` on replay | The loader changed since the bundle was recorded. See determinism.md |
