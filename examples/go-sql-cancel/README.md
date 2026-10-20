# go-sql-cancel

A small Go program that exercises a `database/sql` cancellation race of the kind described in CVE-2025-47907
(GO-2025-3849), together with the pinned toolchains and scripts needed to build it three ways. It is an example for
Hairspring's `process` target: an unmodified Linux program whose scheduling is explored under Hermit.

This directory is a worked example, not a library. Nothing in it is published as an SDK.

## What it is

- **The race.** In the affected releases (before go1.23.12, and go1.24.0 through go1.24.5), cancelling the context of a query while `Rows.Scan` was in
  progress could close the rows without waiting for the scan, so the connection could be handed to the next waiting
  query and its driver could write into a buffer the first `Scan` was still reading. The upstream fix is in go1.24.6
  (golang/go commit `83b4a5db2409`, "database/sql: avoid closing Rows while scan is in progress").
- **A fake driver that reuses a row buffer** (`internal/retaindrv`). Its `Rows.Next` writes each row into one
  per-connection buffer and returns a slice of it, which is legal for a driver but means any early release of the
  connection can overwrite a row that is still being read. There is no real database.
- **A workload that cancels a context during a scan** (`internal/sqlcancel`, `cmd/sqlcancel`). Four workers issue
  queries on a pool limited to one connection. Each query is cancelled by its own canceller while the worker is
  scanning its row.
- **A property** (`go.sql.scan_returns_own_row`, an "always" site). Every `Scan` that returns without error must
  return the row of its own query, not a row from another query and not a mixture of two rows. A false evaluation
  is a `Scan` that returned a row other than its own. Six other
  sites are coverage markers or harness invariants; only the property decides the outcome.
- **A Go shim for the guest event protocol** (`internal/hs`) that reports the sites on standard error as described
  in `docs/guest-protocol.md`. It is an example-internal package.

The program prints one fixed-width summary line on standard output and exits `0` whatever the property result,
`3` if a harness invariant failed, and `64` on a usage error. The property result is carried by the protocol report.

## Honest labels

- **Bug known in advance.** The defect, its upstream fix and the conditions that expose it were public before this
  workload was written. The workload is built around that knowledge.
- **Targeted workload.** The driver, the one-connection pool, the four workers and the cancellers were chosen
  because they put the race in reach. It is not a generic test of `database/sql`.
- **The positive control is assisted.** `-mode=yield` scans into a `sql.Scanner` that copies the value in two
  halves around a sleep. That widens the window on purpose, so a result in this mode says the property and the
  detector work and says nothing about how often the unassisted race is reached. `-mode=std` scans into a `*string`
  and is the unassisted workload.

## Arms

| Arm | Toolchain | Role |
|---|---|---|
| go1.24.5 | official release, unpatched | contains the defect |
| go1.24.6 | official release | fixed release, control |
| go1.24.5 + fix | go1.24.5 built from source with only the non-test `sql.go` and `convert.go` hunks of the upstream fix (`patches/fix-only.patch`) | fixed in `database/sql` only; every other file identical to go1.24.5 (`scripts/check-patched-diff.sh`) |

Archive checksums are in `PINS`. `BUILD-RECORD` has the sha256 of the toolchain files, the patched tree and the
Linux binaries built from each arm. The patch carries the Go Authors' copyright and is BSD-3-Clause
(https://go.dev/LICENSE).

## What you need

- **Go.** go1.24.5 and go1.24.6 (official archives, fetched and verified by `scripts/fetch-go.sh`). The module
  declares no dependencies outside the standard library.
- **To run natively:** any host that can run the pinned toolchains, for example macOS arm64 or Linux x86-64.
- **To run under Hermit:** Linux x86-64 with hardware performance counters (the experiment ran on one bare-metal
  machine; a VM was not tested), the maintained Hermit fork described in `docs/determinism.md`, and the Hairspring CLI. Apple Silicon cannot run
  Hermit. The flags used for the experiment are listed in
  [the write-up](../../docs/showcase-go-sql-cancel-race.md) (section 4).
- For the scripts: `curl`, `shasum`, and a C-free build (`CGO_ENABLED=0`); the race-detector cells in
  `scripts/run-m1d.sh` additionally need cgo.

## Build the toolchains

From this directory:

```sh
scripts/fetch-go.sh                    # download and verify the official archives against PINS
scripts/build-patched-toolchain.sh     # unpack both releases and build go1.24.5 plus the fix
scripts/check-patched-diff.sh          # confirm the patched tree differs from stock only in sql.go and convert.go
```

`scripts/fetch-go.sh` and `scripts/build-patched-toolchain.sh` work on macOS arm64 only: they download and unpack the
`darwin-arm64` archives. On Linux, download the `linux-amd64` archives whose sha256 values are in `PINS` by hand and
unpack them yourself (building go1.24.5 plus the fix also needs the go1.24.5 source archive and `patches/fix-only.patch`).
The sha256 values recorded in `BUILD-RECORD` for the toolchains and for the binaries were produced on darwin-arm64.

Toolchains are unpacked under `.cache/toolchains/`, which is git-ignored. Use a toolchain without touching your
`PATH` by setting `GOROOT` and `GOTOOLCHAIN=local`.

## Run it natively

```sh
export GOROOT="$PWD/.cache/toolchains/go1.24.5"     # or go1.24.6, or go1.24.5-fixonly
export GOTOOLCHAIN=local GOPROXY=off
"$GOROOT/bin/go" test ./...                          # unit tests
"$GOROOT/bin/go" build -o sqlcancel ./cmd/sqlcancel
./sqlcancel -mode=yield -q 100                       # assisted positive control
./sqlcancel -mode=std   -q 1000                      # unassisted workload
```

`-q` is the number of queries per worker (defaults: 1000 for `std`, 100 for `yield`). The summary line reports the
number of queries, successful and failed scans, and counts of successful scans that returned a foreign, torn or
unparsable row; the protocol report on standard error names the sites. `scripts/run-m1d.sh` runs the native and
race-detector cells on macOS across the three toolchains and keeps every run's raw output under `results/` (only the
two summary files are included in this repository); `scripts/summarize-m1d.sh` tabulates counts.

The Rust conformance tests check the workload's output against the reference protocol parser:

```sh
HS_GO="$GOROOT/bin/go" SQLCANCEL_EXPECT=buggy \
  cargo test -p hairspring-props --test go_shim --test go_sqlcancel
```

Set `SQLCANCEL_EXPECT=clean` for go1.24.6 and go1.24.5 plus the fix. Without `HS_GO` the tests use `go` from
`PATH`, and they print a skip notice instead of running if that Go is missing or older than 1.24 (set `HS_REQUIRE_GO=1`
to make that an error; empty, `0` and `false` count as unset).

## Run it under Hairspring and Hermit

Build static Linux binaries with the flags recorded in `BUILD-RECORD`, one per arm:

```sh
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 \
  "$GOROOT/bin/go" build -trimpath -buildvcs=false -ldflags=-buildid= -o sqlcancel ./cmd/sqlcancel
```

The same source built with `-tags hs_violation_only` prints only violation lines on standard error and is used as
a check that the report does not depend on the full event stream.

Then declare the binary as a `process` target in `hairspring.toml` (see the process-target section of the
top-level README and `docs/guest-protocol.md` for the target settings and how a trial is judged) and run
`hairspring test <target> --seed <n>`. A failing seed is bundled and replays with `hairspring replay <bundle>` in
the environment that recorded it. The scheduler settings of the experiment were: `--chaos --max-timeslice=100000
--skid-margin=25 --panic-on-rbc-overshoot --epoch=2026-01-01T00:00:00Z --base-env=minimal --env=GOMAXPROCS=2
--env=GOTRACEBACK=all`, with the engine's equivalent target keys `chaos = true`, `max_timeslice_ns = 100000`,
`skid_margin = 25` and `hermit_args`. The counted runs were direct Hermit runs, not engine runs; see
[`experiment/`](experiment).

## Results

The experiment that used this workload is described in
[the write-up](../../docs/showcase-go-sql-cancel-race.md). In short, on one machine (Ubuntu 24.04.5, kernel
6.17.0-42-generic, Intel Xeon E5-1410 v2) under one pinned Hermit build, the property site was false at least once in
934 of 1,996 rated runs on go1.24.5 (seeds fixed in advance; Wilson 95% [44.61%, 48.99%]; four runs were Hermit failures and are outside the
rated runs) and in 0 of 2,000 on each of go1.24.6 and go1.24.5 plus only the fix, for the same 2,000 seeds. The first
100 hit seeds in seed order were each confirmed by three direct reruns, by the engine at the same seed (in a different
Hermit environment) and by bundle replay. Labels: bug known in advance; targeted workload; environment pinned;
go1.24.5 unassisted and built with the protocol shim (`T2_instrumented`); go1.24.6 a fixed release, compared at arm
level only; go1.24.5 plus the fix a patched toolchain. The evaluator's mechanical code for these runs is
`C1-DONE/R-CRASH+R-CRASH-hermit(u=2,c=0,cp=0)`; the statements above are made under its `OUTCOME-SKID-B` reading,
`C1-DONE/R-HIT+R-CRASH-hermit(u=4,c=0,cp=0)`, which the write-up explains. These are counts of runs in one setup, not
a rate of defects and not a claim about any other driver, Go version or machine. The scripts are in
[`experiment/`](experiment). The binaries' sha256 values are in `BUILD-RECORD`.

`results/` holds the summary files of informational native and race-detector runs on macOS. Those are not Hermit
runs, are not part of the experiment and are not summarised here.

## Layout

- `cmd/sqlcancel` the workload; `cmd/hello` and `cmd/hellosql` small programs whose builds are recorded per
  toolchain; `cmd/hssample` the conformance fixture for the protocol shim
- `internal/retaindrv` the fake driver; `internal/sqlcancel` the workload logic; `internal/hs` the protocol shim
- `patches/` the fix-only patch; `scripts/` fetch, build, check and run scripts
- `experiment/` the drivers, evaluators and Hermit recipe of the counted runs; `results/` summaries of the native runs
- `PINS` archive checksums; `BUILD-RECORD` recorded hashes of toolchains and binaries
