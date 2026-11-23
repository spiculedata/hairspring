# The dhyve backend

## Experimental, not covered by the support matrix, no results published

This backend is off by default. A normal build and every release binary leave it out, and nothing in a default build
mentions it. It makes no statement about any language or program, and `docs/support-matrix.md` does not cover it.

What exists today is a machine check and the smallest shape of a test run: one prepare and then one restore for each
trial. It reports no capability, and nothing it does can back a claim.

### Building it

```bash
cargo build -p hairspring-cli --features dhyve
```

### Checking a machine

```bash
hairspring doctor --backend dhyve
```

This reads a few facts about the machine and prints one line for each prerequisite, then says what is missing and how to
fix it. It reads files and asks `kldstat` and `sysctl` questions with a five second limit each. It never loads or
unloads a kernel module, never starts a virtual machine and never changes a setting.

- On a machine that is not FreeBSD on x86-64 it runs no command, prints that the machine is not supported, and exits 4.
- On FreeBSD on x86-64 it checks the CPU's virtualisation features, that a `vmm` kernel module is loaded, that the two
  programs named by `HAIRSPRING_DHYVE_BHYVE` and `HAIRSPRING_DHYVE_BHYVECTL` exist (with their SHA-256), that no
  virtual machine from an earlier run is left, and the `machdep.panic_on_nmi` setting.
- Exit 0 means the prerequisites it can read are met. It does not show that the backend works.

A build without the feature answers `unknown backend` for this name, as for any other name it does not have.

### The run path

The run path starts virtual machines. Everything here was written against a stand-in for the VM programs and has not been
run on FreeBSD as part of this change.

- **One prepare.** The guest is booted once, up to its first stop. A checkpoint is taken there, the VM is shut down, and
  the files, the values a restore must be given and the instruction pointer the guest resumes at are recorded.
- **One restore for each trial.** The checkpoint is restored under a new VM name, given its own input files (one of them
  holds the trial's seed), resumed through a control socket this process listens on, and read until the guest reports how
  its program ended on the serial log.
- **The program that runs** is the one built into the guest image. The target's `command` is not passed to the guest yet.
- **Faults of the host or the tooling are never findings.** A trial that meets one ends the run with an error and is
  never counted as a finding. The faults are: another run holds the host; a program could not be started; no `vmm` module
  is loaded, or the loaded one is not the one seen at the start; a program or a library does not match its pin; the VM did
  not connect to the control socket or reach its first stop; the control connection ended unexpectedly; the resume
  command was not taken; the VM stopped again after it was resumed; log size bound reached; not enough free space; the
  serial log is missing or empty; the VM ended without reporting an exit status; the VM program did not exit; the input
  files were not as written; the checkpoint could not be made or read, or was not made by the expected module; and a
  second prepare was refused. A line the VM logged is never put in an error text.
- **One VM at a time on a host.** A lock on a file in the state directory is held for the whole of a prepare and of each
  trial. A second run waits, and is then told that another run holds the host. Every VM name has the fixed form
  `vmvx_<stamp>_XB_rNN`. The stamp is the one in the settings, or else one taken from a counter file under the lock, so
  that no name is used twice. The prepare is `r01`, and the restores are `r02` and on.
- **The module is checked before every start.** A VM start may load a kernel module by itself when none is loaded, so
  `kldstat` is asked, under the lock, just before each start. The start is refused unless the module is loaded and is the
  one seen at the prepare.
- **Every wait has a deadline.** A VM that is still there after it is destroyed is asked to end through its process group,
  then killed, and a VM that will not go is an error, not a wait. A VM, its starting process and its control socket are
  ended on every path, a panic included.
- **Bounded logs.** The size of the VM's log and of the serial log is looked at while a trial runs. A trial that passes a
  bound is ended. Until a write rate has been measured on a host the bounds are 256 MiB and 16 MiB, a trial is cut off
  after 100 seconds at most, and at least 2 GiB must be free under the state directory. The last megabyte of a log is
  kept after an error or a finding; a session that keeps everything keeps every log and input file in full.
- **A VM that stops again after it was resumed** is reported as that, and the kind of the stop is recorded. A branch limit
  of 4,000,000,000 applies to every trial; this page does not say that a limit is what stopped a VM.

The backend never loads or unloads a kernel module; an operator does that. It runs unprivileged. The three commands that
need root (start a VM, destroy a VM, checkpoint a VM) run as `sudo -n env -i LD_LIBRARY_PATH=<lib> <program> ...` with the
pinned program and library directory, and the host's `sudoers` must allow exactly those. The only other programs it runs
are the read-only questions of the machine check, `kldstat` and `df -Pk <state directory>`. A start runs under
`cpuset -l <core>` (core 1 unless the settings say 2 or 3). Anything else is refused before it is started: a name of
another stamp, a checkpoint anywhere but the session's checkpoint directory, an option the start does not use, a value
of any option other than the one the builder gives it (a network backend, a disk, a device or a path outside the run's own
directory, for example), and a
program called `kldload` or `kldunload` by name. The two VM programs must be regular files, not links, and may be pinned
by SHA-256 together with the library directory, whose files must all be regular files (a link in it is refused).

Every capability flag is `false`, including replay. Sweeps, replay, branching, bundles that can be replayed, workers and
`hairspring gc` do not use this backend, and a config that names `backend = "dhyve"` is not accepted yet, so the run path
is reached only through the session driver below or through the Rust API. Runs on this backend are never used to back a
claim.

### Settings

| Variable | Meaning |
|---|---|
| `HAIRSPRING_DHYVE_BHYVE`, `HAIRSPRING_DHYVE_BHYVECTL` | The two VM programs, by absolute path. |
| `HAIRSPRING_DHYVE_SUDO` | The program that gives them root. Fixed at `/usr/local/bin/sudo` except in a build with the stand-in programs. |
| `HAIRSPRING_DHYVE_STAMP` | The session's stamp, for example `20261201T0540Z`. With none, a stamp is taken from the counter. |
| `HAIRSPRING_DHYVE_CORE` | The host core a VM is pinned to: 1, 2 or 3. Defaults to 1. |
| `HAIRSPRING_DHYVE_PIN_BHYVE`, `HAIRSPRING_DHYVE_PIN_BHYVECTL`, `HAIRSPRING_DHYVE_PIN_LIB` | SHA-256 of each program, and of the library directory's manifest (one `<sha256>  <name>` line for each file, sorted by name). |
| `HAIRSPRING_DHYVE_LIB` | The directory they load their libraries from. |
| `HAIRSPRING_DHYVE_STATE` | The directory for the lock, the stamp counter, checkpoints and each VM's files. |
| `HAIRSPRING_DHYVE_KERNEL`, `HAIRSPRING_DHYVE_INITRD` | The guest's kernel and initial RAM disk. |

### The session driver

```bash
cargo build -p hairspring-dhyve --features driver --bin dhyve-session-driver
dhyve-session-driver --control 10 --other 5 --seed 1007 --deadline-secs 1410
```

It runs one prepare, then 10 restores given the checkpointed run's own input files (group C) and 5 given another seed file
(group S, the seed one past the first), in the order `CCSCCSCCSCCSCCS`, each cut off after 70 seconds. It stops if the
first restore is not counted, starts a restore only if it can end before the deadline, and refuses a second prepare. It
keeps every log and input file, and writes a steps log and a record for each run under `<state>/records`. It prints one
JSON summary: for each group, how many restores were resumed, how many reported an exit status, and each trial's outcome;
for group C, whether the serial output and the event lines of the restores were `equal` or `differed`. Nothing is compared
across seeds. It exits 0 when the prepare worked, no restore met a fault of the host or the tooling and the session ran to
its end, 1 otherwise, and 2 for a bad command line or a missing setting. It needs its stamp and all three pins
set, and exits 2 without starting anything if one is not. `--deadline-secs` is seconds from its own start, so whoever starts
it hands in what is left of the session's time. Every summary says whether the stand-in programs are built in
(`fake_vmm`), a build that has them refuses to run without `--allow-fake`, and `--build-info` prints only that. It is not
built by a default `cargo build` or by any release: it needs the `driver` feature. It runs the backend's own prepare, restore and judging functions in its own loop
and not through the engine's `run`.

### Testing it on any machine

`cargo test -p hairspring-dhyve` runs the run path against a fake `bhyve`, `bhyvectl` and `sudo` (one small program started
through three links) that speaks the control protocol and can be scripted to misbehave: a stale stop sequence, a dropped
socket, a name that already exists, a resume that is not taken, a stop after the resume, a VM that does not go when it
is destroyed, a `sudo` that cannot be signalled, a destroy that never answers, a module that is not loaded, a panic, a log
that grows past its bound, and more. Those tests show how this crate reacts to each scripted case. They say nothing about a real VM.
