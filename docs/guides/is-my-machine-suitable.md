# Is my machine suitable for the process backend?

The `process` backend runs an unmodified Linux program under Hermit, which schedules its threads from a seed. That
needs a particular kind of machine. The `sim` and `wasm` backends need none of this and run on macOS and Linux (see
[test your logic with wasm](test-logic-with-wasm.md)).

This page says what the backend needs, how to check a machine before you spend time on it, how to read the answers,
and what has and has not been tried. It states what was measured or what the other documents already say. Where
something was not tested it says so and tells you how to test your own machine.

## What it needs

| Need | Why |
|---|---|
| Linux on x86-64 | Hermit runs only there. macOS and Apple silicon cannot run it |
| Hardware performance counters (a CPU performance monitoring unit, PMU) that the kernel can see | Hermit counts hardware events to decide when to switch threads |
| `kernel.perf_event_paranoid` low enough for your user, or run as root | Opening a counter |
| `kernel.yama.ptrace_scope` of 0 or 1, or run as root (2 needs root, 3 disables tracing) | Hermit traces the program with ptrace |
| Linux 6.9 or newer (recommended) | Older kernels take a slower ptrace path. Schedules are not affected; timeouts may be |

These are the same requirements as in the [quickstart](../quickstart.md#what-you-need). The scripts never use `sudo`,
never change a kernel setting and never install system packages.

The only configuration that has been measured is in the [support matrix](../support-matrix.md#the-one-configuration):
Ubuntu 24.04.5 LTS, kernel 6.17.0-42-generic, one bare-metal host with an Intel Xeon E5-1410 v2 (Ivy Bridge) CPU, a
pinned Hermit build, run as root. The results apply only to that configuration. Passing the checks below means the
prerequisites are met on your machine. It does not mean a program has been run under Hermit there, and it does not
extend the matrix to your hardware.

## Check a machine, in this order

Run these on the machine you plan to use, not on your laptop. Each step is cheap, and the first two change nothing.

### 1. The read-only host check

From a checkout of this repository:

```bash
bash scripts/install-hermit.sh check
```

It reads `/proc` and `/sys`, prints one line per prerequisite, and exits 0 when the prerequisites are met or 1 when
they are not. It builds nothing and does not need Hermit to exist yet.

On an Apple silicon Mac (run here):

```text
Hermit host check (read-only: nothing on this machine is changed)
  platform       FAIL  Darwin arm64
result: CANNOT RUN Hermit on this machine:
  - Hermit runs only on Linux; this is Darwin.
```

Inside a `linux/amd64` Ubuntu 24.04 container on Docker Desktop on that Mac (run here, as root; the container is
emulated):

```text
Hermit host check (read-only: nothing on this machine is changed)
  platform       ok    Linux x86_64
  kernel         ok    6.12.54-linuxkit
  perf           ok    perf_event_paranoid = 2 (running as root)
  ptrace         ok    no Yama ptrace restriction
  pmu            FAIL  no hardware PMU registered
  container      note  running in a container: counters and ptrace are often blocked here
result: CANNOT RUN Hermit on this machine:
  - no CPU performance monitoring unit (PMU) is registered under /sys/bus/event_source/devices. Hermit needs hardware counters: use bare metal or a VM that exposes them.
```

Run as an unprivileged user in the same container, one more line appears:

```text
  root           note  the measurements behind Hairspring's claims ran Hermit as root; running it unprivileged was not tested
```

How to read each line:

| Line | `ok` means | `FAIL` or `note` means |
|---|---|---|
| `platform` | Linux on x86-64 | Another operating system or CPU. Nothing else is checked |
| `kernel` | The release string, 6.9 or newer | A `note` if older than 6.9: slower ptrace path, schedules unaffected |
| `perf` | `perf_event_paranoid` does not deny counters to you (running as root counts as ok) | `FAIL` when a non-root user faces a value of 3 or more, or the file is missing |
| `ptrace` | No Yama restriction, or one that allows you | `FAIL` for a scope of 3, or 2 without root |
| `pmu` | The kernel lists a CPU performance monitoring unit under `/sys/bus/event_source/devices` | `FAIL`: there are no hardware counters to open. See the next section |
| `root` | (not printed as root) | A `note`: the measurements ran as root and running unprivileged was not tested |
| `container` | (not printed outside a container) | A `note`: counters and ptrace are often blocked in a container |

Everything under `result:` is a blocker with its reason. The script names a setting that is in the way and leaves the
decision to you: lowering a security setting is a trade-off for the owner of the machine.

### 2. The authoritative probe

Once the check passes, build Hermit with `scripts/install-hermit.sh install` (see the
[quickstart](../quickstart.md#install)), then:

```bash
export HAIRSPRING_HERMIT="$HOME/.local/share/hairspring/hermit/bin/hermit"
hairspring doctor --process-backend
```

Unlike the script, this opens a real hardware counter, so it can find a problem that `/proc` and `/sys` do not show.
Its last line is `result: ok: ...` (exit 0) or `result: CANNOT RUN the process backend on this machine:` followed by
the reasons (exit 4; 1 means a property was violated).

On the Mac (run here, with a build of `main` and no Hermit):

```text
process backend: prerequisites on this machine
  platform       FAIL  macos aarch64
  kernel         note  release could not be read
  hermit         FAIL  not found
  perf counters  FAIL  not checked: no hardware counter was opened
  ptrace         ok    no Yama restriction
  loader / libc  ok    C library version unknown; no loader files could be read
result: CANNOT RUN the process backend on this machine:
  - the process backend needs Linux: Hermit does not run on this operating system. The sim and wasm backends work here.
  - Hermit was not found: HAIRSPRING_HERMIT is unset and `hermit` is not on PATH. Build it with scripts/install-hermit.sh and point HAIRSPRING_HERMIT at the result.
  - no hardware performance counter was opened on this machine (the probe did not run, which happens off Linux x86-64), so the process backend cannot be said to be runnable.
```

In the emulated container (run here, as root, a build of `main`, Hermit not installed there):

```text
process backend: prerequisites on this machine
  platform       ok    linux x86_64
  kernel         ok    6.12.54-linuxkit
  hermit         FAIL  not found
  perf counters  FAIL  no hardware PMU registered (perf_event_paranoid 2)
  ptrace         ok    no Yama restriction
  container      note  running inside docker: counters and ptrace are often blocked here
  loader / libc  ok    ldd (Ubuntu GLIBC 2.39-0ubuntu8.9) 2.39; ld.so.cache sha256 ..., libc sha256 ..., ld.so sha256 ...
result: CANNOT RUN the process backend on this machine:
  - Hermit was not found: HAIRSPRING_HERMIT is unset and `hermit` is not on PATH. Build it with scripts/install-hermit.sh and point HAIRSPRING_HERMIT at the result.
  - no hardware PMU registered: this machine (a virtual machine or container, or an emulated one) cannot provide the counters Hermit needs; lowering perf_event_paranoid will not help
```

The unprivileged user in that container got the same report. (The hashes are elided here; the command prints them.)

How to read the rows that differ from the script's:

| Line | Meaning |
|---|---|
| `hermit` | `ok` shows the path, Hermit's version and its sha256. `FAIL` means it was not found (set `HAIRSPRING_HERMIT`), or the file did not answer `--version` |
| `perf counters` | `ok` means a hardware branch counter was opened. A `FAIL` names the reason: no PMU registered, a refusal with the PMU unknown (`/sys` not visible, as in a chroot or sandbox), or a refusal that is about the setting |
| `loader / libc` | The C library and loader files. A recording is tied to them (see [determinism](../determinism.md)) |

The `ok` forms of the `hermit` row and of the final `result:` line were not run here, because no machine used for this
page had a PMU. The text comes from the source.

### 3. A real run

`hairspring doctor <target>` runs a `process` target twice per seed and compares the runs, and `hairspring test
<target>` explores it. Both need Hermit. Do this before you rely on the machine; see
[running your own software](https://github.com/spiculedata/hairspring/wiki/Running-Your-Own-Software) and the
[quickstart](../quickstart.md#run-a-c-example).

## Why lowering `perf_event_paranoid` does not help when no PMU is registered

`perf_event_paranoid` decides who may open a counter. It cannot create one. When the `pmu` line (or the `perf
counters` line) says `no hardware PMU registered`, the kernel lists no performance monitoring unit at all, so there is
nothing to open whatever the setting. The kernel can even refuse with a permission error before it finds there is no
hardware, which is why older builds of `hairspring doctor` suggested lowering the setting on such a machine. The
current `doctor` checks for a registered PMU first and says so (this was fixed in #198).

If `check` says the PMU exists but `doctor` says the counter was refused, then the setting, a container profile or a
security module may be in the way. Whether and how to change it is the machine owner's decision. To see the setting:
`cat /proc/sys/kernel/perf_event_paranoid`.

## What to try, and what was tried

| Environment | What is known |
|---|---|
| Bare-metal Linux x86-64 host, as in the matrix | The only configuration measured. Supported languages and their limits are in the [support matrix](../support-matrix.md) |
| Docker Desktop on an Apple silicon Mac, `linux/amd64` container | Tried: the container is emulated, and the check printed `no hardware PMU registered` (both scripts, as root and as an unprivileged user) |
| Docker Desktop on an Intel Mac or on Windows | Not tested |
| Cloud virtual machines | Not tested. The [quickstart](../quickstart.md#what-you-need) says ordinary cloud VMs and containers were not tested and often do not expose the counters |
| Bare-metal cloud instances | Not tested |
| A virtual machine that passes the CPU's counters through | Named in the [quickstart](../quickstart.md#what-you-need) as acceptable; not tested |
| GitHub-hosted runners | Not tested |
| Any other CPU, kernel or distribution | Not tested. The matrix makes no claim about other hosts, kernels or Hermit builds |

To test your own machine, run steps 1 and 2 above on it. A `pmu ok` and a `result: ok:` from `doctor` are the first
two hurdles, and nothing here promises more. Then run `hairspring doctor <target>` on a small program and read what it
says.

## Root or not

The measurements behind the support matrix ran Hermit as root. Running it as an unprivileged user was not tested, and
`install-hermit.sh check` prints a `root` note to say so. The kernel settings behave differently for the two: root is
`ok` for any `perf_event_paranoid` value and for a `ptrace_scope` of 2. SECURITY.md has its own advice on running a
server and workers: [SECURITY.md](../../SECURITY.md). This page makes no recommendation of its own.

## What was not run for this page

Hermit was not built or run for this page, and no process target was run: it needs a Linux x86-64 machine with a PMU,
and the machines used here had none. Every `FAIL` line above is real output; the `ok` forms
of the `hermit` and `result:` lines come from the source.
