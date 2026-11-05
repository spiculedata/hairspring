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

What the Hermit installer pins and verifies (the pins are in one file, `scripts/hermit.pin`, which the installer reads and
refuses to run without):

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
  prints the target, the scheduler seed, and where the bundle was written (`.hairspring/bundles/`). A sweep writes one
  bundle for the lowest failing seed and one for each later seed at which a different property failed for the first
  time.
- Ctrl-C stops the command, kills the Hermit processes it started, leaves the run `cancelled` with the trials already
  judged, and exits 130, also when Ctrl-C comes just after the last run finished (that run's result stands as stored). `hairspring confirm` does the same.
- `--format json`, `junit` or `sarif` write the same result for tools.
- `hairspring results` lists stored runs; `hairspring show <run id>` shows one, including the properties it checked.
- A small budget may or may not hit the race. Whether it does depends on the machine and the budget, so a pass with a
  small budget says only that those seeds did not fail. Raise `max_trials`, or run the `fixed` mode for comparison
  (the same program with a mutex).

## The Overview page

With `hairspring serve` running, the web UI opens on the Overview page (the address `/`). The list of runs, with the form
that starts one, is the Runs page at `/runs`. Overview shows only what the server sends:

- **Needs attention**: runs whose state is `failed` or `error` among the newest 20 runs, and the newest stored failures.
  When neither list has an entry it says which runs it looked at; it does not say that anything is fine.
- **Recent runs**: the newest five runs, each with its status badge.
- **Workers**: how many workers the server lists and when one was last heard from. It does not say what they are doing.
- **No runs yet**: when the server has no stored runs, the page lists the next steps from this guide
  (`hairspring targets`, `hairspring init`, `hairspring test <target> --seed 1`, `hairspring replay`). A list or a count
  that could not be loaded is reported by name and is never shown as empty.

The page loads again by itself every few seconds. This holds for every page that follows a live run, including the
property table, the seed explorer's trial table and its strip: no request is made while the browser tab is hidden, the
page refreshes as soon as the tab is shown again, and when requests keep failing a banner says the server cannot be
reached.

A Glossary page at `/glossary` defines the words the pages use (seed, trial, sweep, bundle and so on). On the failure
page, a word with a dotted underline, such as Confirmation, opens its definition when pressed or activated from the
keyboard, with a link to the glossary entry. The definitions are shortened from the wiki Glossary.

## Tabs on the run page

A run page shows the run's facts, its Cancel button and the "what this run shows" panel at the top, and below them four
tabs: **Properties** (the property table and the coverage chart), **Seeds and trials** (a link to the seed explorer),
**Failures** (this run's failure records, with the count on the tab) and **Activity** (the run's events as they
arrive). The chosen tab is kept in the address as `?tab=seeds`, `?tab=failures` or `?tab=activity`; the Properties tab
is the plain address `/runs/<run id>`, which also opens on it. An address with an unknown tab name opens Properties.
Following a tab adds a history entry, so Back returns to the tab before. With the keyboard, Left and Right (wrapping),
Home and End move between the tabs and select the one reached; Tab then moves into the tab's content. The seed explorer
keeps its own address, `/runs/<run id>/seeds`, with its filters. The selected tab is marked by a bold label and a thicker
underline as well as by colour, and on a phone the tab row scrolls sideways instead of wrapping.

The run page also shows the coverage chart (below), so opening a run page makes a request to
`GET /api/runs/{id}/coverage`; it is repeated while the run is live and only while the Properties tab is showing. The
page's event stream (`/api/runs/{id}/events`) stays open for as long as the page is, whichever tab is selected, so the
lines that arrive while another tab is showing are there when Activity is opened. It does not pause while the browser
tab is hidden.

## Coverage chart on the run page

A run page has a **Coverage over trials** section, on the Properties tab, drawn from `GET /api/runs/{id}/coverage`. It has three small charts
that share one axis, the trial number: property ids reached, property ids seen true, and distinct schedules. Each is a
cumulative count of distinct things seen so far, so a line never falls, and each has its own scale starting at zero
because the units differ; the label at the top of each y-axis is the round value the top edge stands for, and the line is
drawn to that scale. A flat stretch means nothing new was seen there; it does not mean nothing is left to find.
Under the charts, **Show as a table** lists the same points as figures.

- The chart draws at most 2,000 points. A longer series is thinned evenly, keeping the first and the last point, and the
  page says **Downsampled** with how many points it shows of how many it received. The figure under each heading (final value and the trial it last rose at) comes from every
  point, so that trial may not be one of the points drawn. The server may already have thinned a
  very long series; the page says that too.
- A run that is still going is refreshed every few seconds, and loaded once more when it ends.
- A run with no judged trial says so instead of drawing an empty chart.

## Side navigation and colour theme

The web UI has a side navigation column with links to Overview, Runs, Failures, Targets and Workers; the link for the
section you are in is marked. On a narrow window (860 pixels wide or less) it becomes a row of links under the top bar.
The top bar has a **Colour theme** switch with three buttons: System (follow the browser or operating system setting,
the default), Light and Dark. The choice is saved in your browser's local storage, so it is remembered per browser and
is not shared between browsers or machines. If the browser blocks storage (some private windows do), the choice still
works but lasts only until you close or reload the page.

To show your organisation's name in the top bar, start the server with `hairspring serve --organisation "Acme Ltd"` or
set `HAIRSPRING_ORGANISATION`. It is plain text of at most 80 characters (no logo, no markup); an empty value, or no
setting, shows nothing, and a name with a line break, another control character or an invisible formatting character
is refused when the server starts. The browser reads it from `GET /api/health`, which adds an `organisation` field
only for a caller with a valid token (or any caller when the server has no tokens), so a probe without a token still
gets just `status` and `version`. A health request that carries a token counts against the same per-client rate
limit as the other endpoints; once that limit is used up the `organisation` field is left out (the answer is still
200), so health cannot be used to guess tokens. A request without a token is never limited.

## Start a run or a sweep from the web UI

With `hairspring serve` running, the Runs page of the web UI has a form that starts a run through `POST /api/runs`. Pick
a target and a run type. A search (`Find a failure`) ends at its first confirmed failure, so it has no rate. A sweep
(`Measure how often`) runs every trial of its budget, continues after a failure and stores every trial, so a rate can
be stated for the trials it ran; a sweep with no failure says only that those trials did not fail.

- The form shows the budget the server will enforce before you start: the target's trial and wall-clock budgets, and,
  for a simulation (`sim`) target only, its step budget. A sweep can take a long time. It is bounded by those budgets,
  and a cancel, or three host failures in a row, ends it early.
- A sweep takes an optional number of trials, from 1 to 10,000 (the server clamps larger values to 10,000). The form
  sends it inside the target's whole budget, so the target's steps and seconds are unchanged. It cannot change the
  step or wall-clock budget.
- The seed is optional, as on the command line.
- Starting a run needs a token with the `run` role. Without one the server answers 403 ("this endpoint requires the
  run role") and the form says the token cannot start runs.
- Once the server accepts the run, the browser opens its page. Confirming a failure is not started from the browser.

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

| Language | SDK | Fixture | Status under Hermit |
|---|---|---|---|
| C | [`sdks/c`](../sdks/c), [c-sdk.md](c-sdk.md) | `examples/language-fixtures/c` | Supported on one tested configuration; see the [support matrix](support-matrix.md) |
| C++ (clang++ and g++) | the C SDK (see c-sdk.md) | `examples/language-fixtures/cpp` | Supported on one tested configuration; see the [support matrix](support-matrix.md) |
| Go | [`sdks/go`](../sdks/go) | `examples/language-fixtures/go`, `examples/go-sql-cancel` | Supported on one tested configuration; see the [support matrix](support-matrix.md) |
| Rust | [`sdks/rust`](../sdks/rust), [rust-sdk.md](rust-sdk.md) | `sdks/rust/fixtures` | Supported on one tested configuration; see the [support matrix](support-matrix.md) |
| Python | [`sdks/python`](../sdks/python), [python-sdk.md](python-sdk.md) | `examples/language-fixtures/python` | Not supported on this build: the probe program did not complete under Hermit |
| Java | [`sdks/java`](../sdks/java), [java-sdk.md](java-sdk.md) | `examples/language-fixtures/java` | Supported on a Java-specific configuration; see the [support matrix](support-matrix.md) |
| .NET | [`sdks/dotnet`](../sdks/dotnet), [dotnet-sdk.md](dotnet-sdk.md) | `examples/language-fixtures/dotnet` | SDK and fixture available. Not supported on this build: the probe program did not complete under Hermit |

"Supported" means the language passed our acceptance test on one configuration (Ubuntu 24.04, one Xeon host, pinned
Hermit build) and was not tested elsewhere. The [support matrix](support-matrix.md) has the details and the exact
configuration; the results apply only to that configuration. Java is supported only on its own configuration:
`skid_margin = 35` (the other languages: 25), the JVM flags `-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1
-XX:-UsePerfData -Xshare:auto`, Temurin 21.0.12.1, and a reduced fixture (50 iterations per worker instead of 1,000). A
Java replay matched only from the same paths as the recording (JDK install, program jar and working directory); see [Replay and paths](../sdks/java/README.md#replay-and-paths).

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
