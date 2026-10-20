# Experiment scripts for the glibc lost-wakeup ladder

These are the drivers, evaluators, build scripts and patches that produced the numbers from the build on the test
machine onward in
[Pinning glibc to test a lost wakeup: what we could and could not measure](../../../docs/showcase-glibc-lost-wakeup.md);
the first build-check script and the container wrapper are not published (see below).
They are published as they ran, with only the changes listed below. They were written for one dedicated Linux x86-64
test machine and assume its layout; read them before running them anywhere.

Licences: the scripts are Apache-2.0, like the rest of this repository. `patches/positive-control.patch` and
`patches/positive-control-p2.patch` are diffs against glibc's `nptl/pthread_cond_wait.c` and carry glibc context lines
(LGPL-2.1-or-later). `patches/hermit-25781c5-replay-backport.diff` is a diff against Hermit (BSD-3-Clause).

## What each file did

| File | Step in the write-up | What it does |
|---|---|---|
| `tools.sh` | all | read-only preflight: tool versions, the exact gcc version, kernel headers, the Hermit binary's sha256, free space |
| `x1b.sh` | 5.1 (build on the test machine) | builds private bison/m4, the U, C and P1 glibc trees into sysroots, links the smoke program and the workload statically, checks isolation (link trace, ELF), plain runs, Hermit smoke runs, the timing probe, the U/P1 member comparison |
| `condsmoke.c` | 5.1 | the two-thread signal/wait smoke program used by `x1b.sh` |
| `x1c.sh` | 5.1 (planted builds) | builds and pins the 20 planted and 2 violation-only workload builds, rebuilds them once to compare bytes, runs the exit-status probe under Hermit and an ungraded P1 cell |
| `x2.sh`, `x2_eval.py` | 5.2 (native reference) | runs U and C alternately and P1 in its own cell, natively; the evaluator classifies runs, computes Wilson and exact bounds, tallies the state-dump shapes and prints the outcome |
| `guard.sh` | 5.2, 5.5 | sourced by `x2.sh` and `glg.sh`: refuses to start work when a stop file, the time budget, a size cap or a foreign heavy process says so; signal handling |
| `x1d.sh` | 5.3 (P1 probe) | 60 runs of the P1 binary under Hermit, recorded only |
| `gp2-in.sh` | 5.4 (P2 container gate) | inside an `ubuntu:24.04` container: builds U and P2, compares `libc.a` members, and grades the P2 loop in the disassembly (pause count, loop order, conditional branches per pass) |
| `x1e.sh` | 5.4 (P2 build and probe) | builds P2 on the test machine with the same recipe, the same disassembly gate, links and pins three binaries, rebuilds once, then runs the 60-run P2 probe under Hermit |
| `glg.sh`, `glg_eval.py`, `fpx.py` | 5.5 (scheduler gates) | the determinism, timing and detector gates under Hermit (part A; part P was never run); `fpx.py` computes the schedule fingerprint from Hermit's summary JSON |
| `patches/positive-control.patch` | P1 | the 10 ms `clock_nanosleep` hunk |
| `patches/positive-control-p2.patch` | P2 | the 40,000-iteration PAUSE busy-wait hunk (blob `e58d3b83`) |
| `patches/hermit-25781c5-replay-backport.diff` | all Hermit runs | the replay change applied to Hermit `25781c5` (see "Building the pinned Hermit") |

Internal step names appear in file names, comments and printed outcome codes; they map to the write-up as follows:
G0 = first build check, X1b = build on the test machine, X2 = native reference, X1c = planted builds, X1d = P1 probe,
G0p2 = P2 container gate, X1e = P2 build and probe, GLG (part A) = scheduler gates, GLG part P / G1 = the
positive-control gate that never ran, C1/C2 = the counted runs that never ran. Comments that say "box" mean the test
machine, "Mac" the workstation that prepared and evaluated the runs, and "spec section" or "design N.N" refer to the
pre-registered specifications, which are not published; every rule they fixed is implemented in these files.

## Changes against the files that ran

Every file was extracted from its pre-registered specification byte for byte, and its sha256 checked against the value
recorded there before the run. The published copies differ only as follows.

| File | sha256 as run | sha256 as published | Changes |
|---|---|---|---|
| `x2.sh` | `4737a08fc5b3eacf4a230f4c88c18ea5b7e6bfec7bbede5ec45cd1cdc77d008e` | `029d68e23ff15d6b86494227acb65bf16b4a95e866988f07f05d00bd58b8d4ca` | the build work-dir path replaced by `/path/to/x1b-glibc-20261017T1441Z` with an `EDIT:` comment |
| `x2_eval.py` | `0bacbfae6369ad04c3f902e070191d0a6fbccdaa019b743984e222771e15e002` | same | none |
| `guard.sh` | `06d83a884adbd5c4b99787534e3cadd0917ec67f96df19da61e07b176f7311fc` | same | none |
| `glg.sh` | `4f024a0386e74a6f6b454f6720fb9e228199c0ed260b5e8b9b7d27f7e089173c` | `6d530dc36610ee99a036661c0d707c8adba6b1a47cbe36fec566bfa0fc209773` | Hermit binary path and the two work-dir paths replaced by `/path/to/...` with `EDIT:` comments; four comments reworded to remove process wording ("separately signed", "reviewer-confirmed", "DATA-DEPENDENT"), e.g. "a separately signed file, DATA-DEPENDENT: P2" became "a separate file, never filled: PART P was never run" |
| `glg_eval.py` | `c1f53fc00eda6c3da8e574f13f3eeeb987e36cfb88d7c9bbde253f41edd70b7d` | `72a743f83e8f61fe9c64a95141d3a530393188e9563d55a2086ebe8030b985ea` | three comment/docstring edits that named people or review roles; no code changed (its selftest still prints both lines, 2,592 chain combinations) |
| `fpx.py` | `5406f1422dc3efd39285b2d9bddfaebc8b440c994d7e410b8be26e48181e19a2` | same | none |
| `x1b.sh` | `19e3ce523860a608b837bc14c4ce89a9fed99371921276757db5ba0a9a2fb856` | `038d526b519e818047f51cbbd14c1225db31dda3e7992421a7e1339191a8b9c0` | Hermit path replaced; the home directory in two `df` calls replaced by `"$HOME"` (the job ran as that user, so the value is the same); four comment edits that named review roles |
| `x1c.sh` | `cc7ed1ebbb723756f0786bf7454bf1ca92a992c420a31016b85a4ccce4d5941d` | `ad65b8890001a62eace26ca842ad23c37f5d8d7afa46521a4af2f4a119f9f51b` | Hermit path and build work-dir path replaced; home directory in three `df` calls replaced by `"$HOME"`; one comment edit |
| `x1d.sh` | `93bd87591c51a0f21dee4f1fbdb5cdd46962d4ae4ab475aa006b8edab2292b95` | `2b10d508f0368a7237b6a08caaa06caf5b56c12ebc95b975c8d51e6eb63f7d4e` | Hermit path and build work-dir path replaced; home directory replaced by `"$HOME"` in three `df` calls (the free-space check in `ok_to_start`, the snapshot, the `FREE` gate); one comment edit |
| `x1e.sh` | `0920eb0cde2b1cb9f422fbbd9976e3ccc986887724b89c849a20c256298e41b3` | `00dec266e7ea0f8c09628a297b5886f010d06856c717af7ea729b2ff506cba06` | Hermit path and build work-dir path replaced; home directory replaced by `"$HOME"` in three `df` calls (the free-space check in `ok_to_start`, the snapshot, the `FREE` gate); two comment edits |
| `gp2-in.sh` | `ae20c7621f252f655612fbf90e26dd19a9fa9059a2a89d3a44b708e90b38521a` | `e0422b21d6bf6406e38f8e3603082ee81c8f70ff3936016c66f8ae0393e0a17f` | two comment edits (the unpublished host-side wrapper) |
| `tools.sh` | `b3da790fa69843df1c448684a5d3b4854c8957951bcde540b33b889348c4a8fa` | `143e66242f09fe52c123e5978ac43eb7ba2289734702ac8c402553ca779587e9` | Hermit path replaced; home directory replaced by `"$HOME"`; two printed headings reworded |
| `condsmoke.c` | `7e8612c4107be88ba5b561c045291ea7acbdb5364da168401880b4bdbf7aa1f1` | same | none |
| `patches/positive-control.patch` | `aca144dad283c67ccd0d0e372c804c0356fc0547e27ff4d1f5e072f2f4094b66` | same | none |
| `patches/positive-control-p2.patch` | `edb50446501aaf479b2859332a1db5131444eed0c74f4922e3c3379eab7470ec` | same | none |
| `patches/hermit-25781c5-replay-backport.diff` | `1f5d06401ccd8ed7adfa3f0dccd4426640a64c97def169bbec82db1c16a8e957` | same | none |

`bash -n` is clean on every published shell script.

**Not published:** the stub tests (they read the private archives of the runs), the host helpers that checked the
machine was idle and listed leftover processes (they name other jobs on a shared machine), the container wrapper of
the P2 gate (it only drove Docker), the G0 build script (superseded by `x1b.sh`), and the specifications themselves.

## What you must edit before running

Search for `EDIT:`. You need: the path of your Hermit binary (and, unless you rebuilt it bit for bit, its sha256 in
`HERMIT_SHA`/`HSHA`, see below), and the work directories of the build scripts. The scripts also assume they run as a
user named `ubuntu`: the root-owned-file scans (`rootowned` in `x1b.sh`, `x1c.sh`, `x1d.sh`, `x1e.sh`, `glg.sh`:
`find ... ! -user ubuntu`) and the leftover-process scan in `guard.sh` (`$2!="ubuntu"`) hard-code that name; replace it
with your user. `x1b.sh`, `x1c.sh`, `x1d.sh` and `x1e.sh` also check the workload tarball's `PINS` file with
`grep -qx 'SDK_COMMIT=a4131e49...'` and `grep -qx 'SDK_TREE=f5d13d77...'` (for example `x1b.sh` line 176). This
example's `PINS` names a different, later C SDK commit, so a tarball of this example fails that check (`CK-FAIL`)
until you replace the two pinned values with the ones in your `PINS`. The reuse gates compare every input with
the sha256 recorded in our run; with your own builds those gates will stop until you replace the pins with your own
values. That is intended: the scripts refuse to run on inputs they were not pinned to.

## Security notes for running the drivers

The scripts are published as they ran and were written for one dedicated, otherwise idle Linux x86-64 test machine.
They were not hardened for shared or production hosts. Read them first, and note:

- **Privilege.** `x1b.sh`, `x1c.sh`, `x1d.sh`, `x1e.sh` and `glg.sh` run every Hermit call as root through `sudo -n`
  (the `timeout`, `env -i` and `taskset` wrapper sits inside `sudo`, so Hermit and the guest run with root privileges).
  They also run `sudo -n /bin/true` as a gate. Everything else, including all builds, runs as the invoking user.
  `x2.sh` and `x2_eval.py` use no `sudo` and no Hermit. Use a disposable machine (a VM was not tested for Hermit
  runs), not a workstation. `sudo -n timeout ... env ... hermit` needs a passwordless `sudo` rule that covers
  `timeout`, which amounts to unrestricted root for that user.
- **Fixed path under `/dev/shm`, executed as root.** In `x1b.sh`, `x1c.sh`, `x1d.sh` and `x1e.sh` the Hermit work
  directory is hard-coded, with no `EDIT:` marker: `H=/dev/shm/x1b-glibc-$STAMP` (`x1c`, `x1d`, `x1e` likewise,
  `STAMP` being a script argument). The scripts `mkdir -p $H`, create `$H/hout` and `$H/runs/<id>/tmp` with mode
  `1777`, copy the guest binaries into `$H`, compare them with `cmp`, and then run them as root through `sudo -n`.
  On a host with other users, someone could pre-create that guessable path, swap a guest binary (for example
  `condlock-U`) after the `cmp` check and have it run as root, or plant a symlink at `hout/<id>.summary.json`. The
  same applies to the `1777` directories of `glg.sh` (a per-run temp directory) and to the `--tmp` directories. The
  only safe setting is a single-user, throwaway machine; do not run the Hermit steps on a shared host. Root-owned
  files remain in those directories after a run and need `sudo` to remove.
- **Hard-coded user name.** The root-owned-file scans (`find ... ! -user ubuntu`) and the leftover-process scan in
  `guard.sh` (`$2!="ubuntu"`) assume the user is `ubuntu`. With another user name they report your own files as
  foreign, or miss real ones. Replace the name before running (see "What you must edit before running").
- **Deleting and killing.** The scripts delete only scratch files named inside their own work directory (for example
  `rm -f "$t/cb.a" "$t/cb.b"`) and none runs `rm -rf`. The work directory `R` is the first argument and `HERMIT` is
  edited in the script; `H` is fixed (see above). A wrong value changes where files are written, and `chmod 1777`,
  `chmod 555` and `chmod 755` then apply there. Check these first. The scripts never kill processes; the comments mention a manual
  `sudo -n kill` of leftover process ids that you would do by hand after reading the listing.
- **Reading outside the work directory.** The snapshots list `/tmp`, `/dev/shm` and `/var/crash`, hash
  `/etc/ld.so.cache` and read `/proc/mounts`, `/proc/loadavg` and process listings (`ps -eo ...args`). A process
  listing can include other users' command lines, and the scripts write matching lines to `procs-now.txt` and to their
  notes. The listing files are written under the work directory, so treat that directory as sensitive and do not
  publish it.
- **Network and packages.** `gp2-in.sh` is meant to run in a throwaway `ubuntu:24.04` container as root and runs
  `apt-get update` and `apt-get install` there, which needs network access. The build scripts build from inputs you
  supply (glibc source trees, GNU m4 and bison tarballs, the workload tarball), unpack them with `tar -xf`, and compare
  them with the sha256 values recorded in the scripts; none of them downloads anything. Only unpack archives you
  built or fetched and verified yourself, and inspect archives with `tar -tvf` before extracting; symlink members can
  place later members outside the extraction directory. Check each input hash against your own source before replacing a pin.
- **Resource use.** The runs pin work to particular CPU cores (`taskset`), run at `nice -n 10`, cap file size with
  `ulimit -f`, and refuse to start when the free-space, time-budget or stop-file checks say so. They are still
  long and CPU-heavy; do not run them on a machine other people depend on.

The Hermit binary run as root executes the guest program with `--chaos` scheduling and writes its summary under the
directories above. Build it yourself from the recipe below rather than running a binary you cannot verify.

## Building the pinned Hermit

The Hermit binary used for every run had sha256
`666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a` (`hermit 0.4.1 (2026-10-15)`). It was built as follows:

```sh
git clone https://github.com/spiculedata/hermit && cd hermit      # a fork of rrnewton/hermit
git checkout -b replay-backport 25781c5c48d47c316324fd912b7dcafa394c339a
git cherry-pick -x 4a2825df7b2b0fb5002aa55af7be74b9f640706f 66863124ab8cc4f91418619552c494c7ea2803b6
git diff 25781c5c48d47c316324fd912b7dcafa394c339a --stat         # 2 files, +87/-69 (detcore/src/lib.rs, detcore/src/tool_local.rs)
cargo build --release -j3 -p hermit --bin hermit                 # toolchain from rust-toolchain.toml: nightly-2026-07-12
```

Our branch head was `956358de9dd72d005028f008db1e7698b5b40209` (a fresh cherry-pick gives a different head, because
the commit dates differ; the diff matches): `git diff 25781c5c4..HEAD` equals
`patches/hermit-25781c5-replay-backport.diff`. The compiler was `rustc 1.99.0-nightly (be8e82435 2026-07-11)`. Whether a
rebuild gives the same sha256 was not tested; expect a different hash, and treat seeds, fingerprints and timings from
our runs as specific to our binary.

All Hermit runs used root through `sudo -n`; running as an unprivileged user was not tested in this work.
