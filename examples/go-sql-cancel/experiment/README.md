# Experiment scripts for the Go database/sql cancel-race counted runs

These are the drivers, evaluators and helper libraries that produced the counted numbers in
[the write-up](../../../docs/showcase-go-sql-cancel-race.md): the 6,000
counted runs (three arms, 2,000 seeds each), the 100 direct reruns and the 100 engine confirmations. They are published as
they ran, with only the changes listed below. They were written for one dedicated Linux x86-64 test machine and assume
its layout; read them before running them anywhere.

Licences: the scripts are Apache-2.0, like the rest of this repository. `patches/hermit-25781c5-replay-backport.diff` is a
diff against Hermit (BSD-3-Clause). The Go fix-only patch is in the example (`patches/fix-only.patch`, BSD-3-Clause, the
Go Authors' copyright).

## Security notes for running the drivers

The published scripts are the as-run copies, and they were run on a machine with a single local user. They are not
hardened for a shared machine. Read this before running them anywhere else.

- **The engine runs as root against a file in a world-writable directory.** `c2e.sh` writes `$R/cfg/ge.toml` into
  `cfg/` (mode 1777) without checking the result of the write, hashes it for the log only, and then runs the engine as
  root through `sudo -n` with `--config "$R/cfg/ge.toml"`. The target commands in that file are run by the engine, so
  another local user who can plant or swap `ge.toml` (or the other files in `cfg/`) gets code run as root.
- **Summary files and temporary directories sit in 1777 directories with predictable names.** `c1.sh` and `c2e.sh`
  write Hermit's summary files into `sum/` and give each run a temporary directory under `tmp/`, both mode 1777 and
  named after the run identifier, and Hermit runs as root there. Another local user could plant those names in advance
  and force Hermit failures or forge the summary files that the schedule fingerprints are read from. The kernel settings
  `fs.protected_regular` and `fs.protected_symlinks` reduce this but do not remove it.
- **What to do instead.** Run the drivers only on a machine that has no other local users. Or create `cfg/`, `sum/`
  and `tmp/` as root-owned directories with mode 0755 before the run (as `c2e.sh` already does for `e/`) and check the
  sha256 of `ge.toml` against the logged value before each engine call; both need edits to the scripts that the
  published copies do not have, and an edited script no longer has the hash in the table below.
- **Paths with a double quote are unsupported.** `c2e.sh` builds the TOML with `printf`, so a work directory or binary
  path containing `"` produces a broken or different configuration.

## What each file does

| File | Role | What it does |
|---|---|---|
| `bin/c1.sh` | counted runs, direct confirmation | `count`: one window of counted runs, each run a direct Hermit run of one arm's binary at one seed (three arms per seed, starting arm rotating). `confirm`: three more direct runs of each of the first 100 hit seeds. Writes one row per run to `runs.tsv` |
| `bin/c2e.sh` | engine confirmation | for each of the first 100 hit seeds: one `hairspring test` at that seed and, when exactly one bundle was written, one `hairspring replay` of it. The engine is used here only; it never counts a hit |
| `bin/c1_eval.py` | evaluator as run (rev 1a) | the pre-registered rules: next cells, infrastructure halts, projected-time checks, hit counting per site, Wilson intervals, the outcome table. Prints the mechanical `OUTCOME` line. The box ran this file in every window |
| `rev1b/c1_eval.py` | evaluator, verdict side (rev 1b) | adds `verdict --skid-b`, which prints the mechanical `OUTCOME` line first and then `OUTCOME-SKID-B`, reading Hermit failures of "shape B" as the skid ledger (see the write-up, section 5.5). Used on the workstation only, over the archived windows. Put it next to `g_eval.py` and `sclib.py` to run it |
| `bin/g_eval.py`, `bin/sclib.py`, `bin/fpx.py` | shared libraries | the run classifier, the statistics (Wilson interval, exact zero bound, median) and the schedule fingerprint, unchanged from the earlier determinism-and-controls step |
| `bin/facts.sh` | fact reader | reads each run's standard output and standard error in pure bash and records facts only; classes are assigned afterwards by `sclib.py` |
| `bin/guard.sh` | run guard | refuses to start work when a stop file, the time line, a size cap or a foreign heavy process says so; signal handling |
| `patches/hermit-25781c5-replay-backport.diff` | Hermit | the replay change applied to Hermit `25781c5` (see "Building the pinned Hermit") |

Not published: the idle check, the preflight listing and the leftover scan (they name other jobs on a shared machine), the
earlier step's driver `g.sh` (its three Hermit functions are byte-identical to `c1.sh`'s, which is the version that
ran), the specifications, and every run record (see below).

Comments in `c1.sh` and `c2e.sh` that cite `SCRIPTS/README.md` mean this file (`experiment/README.md`).

Internal step names appear in comments and in printed outcome codes: C1 = the counted runs, C2a = the direct reruns,
C2b = the engine confirmations, GOG = the earlier determinism and control gates (G0, G1, G2). Comments that say "box"
mean the test machine, "Mac" the workstation that prepared and evaluated the runs, and "spec" or "design N.N" refer to the
pre-registered specifications, which are not published (comments in `sclib.py` and `g_eval.py` cite one of them by the
name GO-SQLCANCEL-DESIGN; that document is not part of this repository); every rule they fixed is implemented in these
files.

## Changes against the files that ran

Each file was copied from the fifth window's archive copy (`bin/`; the same sha256 values are recorded in the run record
of every window) and the rev 1b evaluator from the workstation scratch directory that produced the final verdict. The
"as run" column is the value each window's run record gives. The published copies differ only as follows.

| File | sha256 as run | sha256 as published | Changes |
|---|---|---|---|
| `bin/c1.sh` | `b89b31d9b448b7b9ca80adadef94f3f42e2278bc84591cc38d9ff4e5737971b3` | `9450be0718bb38b4d6fbcd328a8af1c0d210d774d36eea1ad9c27966fa8cc7fd` | the Hermit path replaced by `/path/to/hermit` with an `EDIT:` comment; one comment reworded (it named the reader of each window's code) and one printed string changed (the `steps.tsv` text, which named a person as the one who could raise the cap, now reads "unless the cap is raised"; as-run line 180, published line 181). The pinned Hermit sha256 on the next line is unchanged |
| `bin/c2e.sh` | `c53c12fd10d6139fff66b67d31cc6109d60c7f6e1f49db183d83724d86b7fdbc` | `1f0ca191df58736ade79fa977f773f13214596d71c7601eb9d7ee2c3e4dcad2f` | the Hermit path replaced as above |
| `bin/c1_eval.py` | `f319d7010e3b9690e2e561f5039dcfbbbcde942c1847e78bd66b03b80733d660` | `12437ced9ce6d301771d285c509f797873ef26ddcc321153eef3d735b7b0eddf` | two header comments reworded (one adjective removed, one verb changed); no code changed; its selftest prints the same line (9,216 outcome combinations) |
| `rev1b/c1_eval.py` | `1def4771e4cdbbde53336f6efb4b7cc6257e0abb4bd066c7797c3835dc71585a` | `f15602d00cfa9575e90bdd2296b79b6a28647504eba256f4dd3ba0a9d8361a0c` | the same two header comments; two comments and one fixture note reworded; **two printed strings changed** (the shape-B heading now says "rule fixed 2026-10-18T01:17Z" in place of a process label, and the closing line no longer names a person: "the mechanical code may be accepted as final instead"). No rule, count or code changed |
| `bin/g_eval.py` | `736278c975b667012d35ae22815ba4a0503b77932af937d228d01e4c0e9b209b` | `5423f0d60308bc152b91bf9fc97135dc5b502123a9b62022b505bec183aabc43` | four comments reworded (a person's name removed from budget remarks); no code changed |
| `bin/sclib.py` | `4a736cb368b56f78064e069772ea68f64446325b062cd5972cd226c6befac724` | same | none |
| `bin/fpx.py` | `5406f1422dc3efd39285b2d9bddfaebc8b440c994d7e410b8be26e48181e19a2` | same | none |
| `bin/facts.sh` | `110752488d774aed6703ce9ead3ccc381d9f3df9c57828ee5ce9a23b0e34289a` | same | none |
| `bin/guard.sh` | `06d83a884adbd5c4b99787534e3cadd0917ec67f96df19da61e07b176f7311fc` | same | none |
| `patches/hermit-25781c5-replay-backport.diff` | `1f5d06401ccd8ed7adfa3f0dccd4426640a64c97def169bbec82db1c16a8e957` | same | none |

`bash -n` is clean on the four shell scripts, every Python file parses, and the selftests print their expected lines on
the published copies: `selftest ok: statistics anchored, classify 5120 combinations`, `selftest ok: 3888 chain
combinations` (`g_eval.py`) and `selftest ok: statistics anchored, plan, halts, hits and Hermit failures, 9216 outcome
combinations` (both `c1_eval.py` files). (Checked on the workstation on 2026-10-19.)

Rev 1a and rev 1b are one file apart. Rev 1b changed the `verdict` mode (the `--skid-b` flag) and `analyse()`, and added the
`SKIDB_*` constants, `skid_b_run`, `leftover_clean`, additions to the selftest, and comments and docstrings. The box-side
modes (`next`, `halts`, `probe`, `pausehours`, `prevcheck`, `hits`) are the same in both.

## What you must edit before running

Search for `EDIT:`: the path of your Hermit binary in `c1.sh` and `c2e.sh`. The scripts also assume they run as a user
named `ubuntu`: the leftover-process scan in `guard.sh` (`$2!="ubuntu"`) hard-codes that name; replace it with your user.
The gates compare every input with the sha256 recorded in our run (the Hermit binary, the three workload binaries, both
evaluators' selftests, and, for `c2e.sh`, the engine binary); with your own builds those gates stop the driver until you
replace the pins with your own values. That is intended: the scripts refuse to run on inputs they were not pinned to.

## Directory layout the drivers expect

A fresh work directory `R` in a tmpfs that is not mounted `noexec` (we used a directory under `/dev/shm`) holding `bin/`
(these files, plus the engine binary `bin/hairspring` for `c2e.sh`), `cfg/` (created beforehand with mode 1777, for
`c2e.sh`; see the security notes above), `g/` (the three binaries named `u-shim`, `c-shim`, `cp-shim`), and, for windows
after the first, a `prev/` bundle built by `c1_eval.py prepare` from the earlier windows. `c1.sh` creates `out/`,
`runs/`, `sum/` and `tmp/` itself;
the directories Hermit writes into are made mode 1777 because Hermit runs as root. Root-owned summary files remain in
`sum/` afterwards. The launch lines are in the first comment lines of `c1.sh` and `c2e.sh`.

## Building the pinned Hermit

The Hermit binary used for every run had sha256
`666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a` (`hermit 0.4.1 (2026-10-15)`). The recipe:

```sh
git clone https://github.com/spiculedata/hermit && cd hermit      # a fork of rrnewton/hermit
git checkout -b replay-backport 25781c5c48d47c316324fd912b7dcafa394c339a
git cherry-pick -x 4a2825df7b2b0fb5002aa55af7be74b9f640706f 66863124ab8cc4f91418619552c494c7ea2803b6
git diff 25781c5c48d47c316324fd912b7dcafa394c339a --stat         # 2 files, +87/-69 (detcore/src/lib.rs, detcore/src/tool_local.rs)
cargo build --release -j3 -p hermit --bin hermit                 # toolchain from rust-toolchain.toml: nightly-2026-07-12
```

Our branch head was `956358de9dd72d005028f008db1e7698b5b40209`; `git diff 25781c5c4..HEAD` equals
`patches/hermit-25781c5-replay-backport.diff`. The compiler was `rustc 1.99.0-nightly (be8e82435 2026-07-11)`. Whether a
rebuild gives the same sha256 was not tested; expect a different hash, and treat seeds, fingerprints and timings from our
runs as specific to our binary. Every Hermit run used root through `sudo -n`; an unprivileged run was not tested here.

## What is not published

The raw run records: `runs.tsv` of each window (one row per run), every run's standard output and standard error, the
`steps.tsv` driver logs, the verdict outputs, the engine's bundles and database. A reader can check our method and repeat
it, but cannot re-evaluate our rows.
