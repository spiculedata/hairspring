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
- **Faults of the host or the tooling are never findings.** A trial that meets a fault of the host or of the tooling ends the run with
  an error. It is never counted as a finding. The faults are: another run holds the host, a program could not be started,
  the VM did not connect to the control socket or reach its first stop, the control connection ended unexpectedly, the
  resume command was not taken, branch limit reached, log size bound reached, not enough free space, the serial log is
  missing or empty, the VM ended without reporting an exit status, the input files were not as written, and the
  checkpoint could not be made or read.
- **One VM at a time on a host.** A lock on a file in the state directory is held for the whole of a prepare and of each
  trial. A second run waits, and is then told that another run holds the host. VM names are made from a stamp the lock
  hands out from a counter file, so no name is used twice; they have the form `vmvx_<stamp>_XB_rNN` with the digits of a
  fixed width, which is the form the loaded module's name check accepts.
- **Bounded logs.** The size of the VM's log and of the serial log is looked at while a trial runs. A trial that passes a
  bound is ended. Until a write rate has been measured on a host the bounds are 256 MiB and 16 MiB, a trial is cut off
  after 100 seconds at most, and at least 2 GiB must be free under the state directory.
- **A branch limit** of 4,000,000,000 applies to every trial. A trial that runs into it is reported as such and not as a
  hang.

The backend never loads or unloads a kernel module; an operator does that. It runs unprivileged. The three commands that
need root (start a VM, destroy a VM, checkpoint a VM) run as `sudo -n env -i LD_LIBRARY_PATH=<lib> <program> ...` with the
pinned program and library directory, and the host's `sudoers` must allow exactly those. The only other programs it runs
are the read-only questions of the machine check and `df -Pk <state directory>`. Anything else is refused before it is
started, and a program called `kldload` or `kldunload` is refused by name.

Every capability flag is `false`, including replay. Sweeps, replay, branching, bundles that can be replayed, workers and
`hairspring gc` do not use this backend, and a config that names `backend = "dhyve"` is not accepted yet, so the run path
is reached only through the session driver below or through the Rust API. Runs on this backend are never used to back a
claim.

### Settings

| Variable | Meaning |
|---|---|
| `HAIRSPRING_DHYVE_BHYVE`, `HAIRSPRING_DHYVE_BHYVECTL` | The two VM programs, by absolute path. |
| `HAIRSPRING_DHYVE_SUDO` | The program that gives them root. Defaults to `/usr/local/bin/sudo`. |
| `HAIRSPRING_DHYVE_LIB` | The directory they load their libraries from. |
| `HAIRSPRING_DHYVE_STATE` | The directory for the lock, the stamp counter, checkpoints and each VM's files. |
| `HAIRSPRING_DHYVE_KERNEL`, `HAIRSPRING_DHYVE_INITRD` | The guest's kernel and initial RAM disk. |

### The session driver

```bash
cargo build -p hairspring-dhyve --features driver --bin dhyve-session-driver
dhyve-session-driver --control 10 --other 5 --seed 1
```

It runs one prepare, then 10 restores given the checkpointed run's own input files, then 5 restores given a different
seed file, and prints one JSON summary: for each group, how many restores were resumed, how many reported an exit status,
and each trial's outcome; for the first group, whether the serial output and the event lines of the restores were `equal`
or `differed`. Nothing is compared across seeds. It exits 0 when the prepare worked and no restore met a fault of the host
or the tooling, 1 otherwise, and 2 for a bad command line or a missing setting. It is built only with the `driver` feature
and is no part of any default build.

### Testing it on any machine

`cargo test -p hairspring-dhyve` runs the run path against a fake `bhyve`, `bhyvectl` and `sudo` (one small program started
through three links) that speaks the control protocol and can be scripted to misbehave: a stale stop sequence, a dropped
socket, a name that already exists, a resume that is not taken, the branch limit, a log that grows past its bound, and
more. Those tests show how this crate reacts to each scripted case. They say nothing about a real VM.
