# DS fixtures: small programs for multi-process and kill-and-restart experiments

These are experiment fixtures, not product features. They are tiny C, Go and Rust programs that stand in for the
processes of a small distributed system (a ring of three key-value servers with two clients, and a single node that a
supervisor kills and restarts), so that an experiment can later ask how such a topology behaves when run under a
deterministic scheduler.

**Nothing in this directory has been run under Hermit.** Everything here was built and tested natively (macOS and
Linux). Nothing here is a support claim for any language, platform or topology; the language support statements stay
exactly as `docs/support-matrix.md` says. A native pass shows that a program does what its description says. It says
nothing about any other environment.

## What is here

| Program | Language | What it is |
|---|---|---|
| `dsl` | C | The launcher. Starts one trial's processes, drives them to completion, merges and checks the history, prints one result line. In crash mode it also kills and restarts a node on a plan that is a pure function of a seed. |
| `dskv` | C | A key-value server. Modes `epoll`, `timeout` (heartbeats and suspicion, with an optional second thread) and `block` (blocking `accept`/`recv`/`send`, no replication). |
| `dscli` | C | A client with a fixed operation list. Records an invoke/return history ordered by a shared sequence counter. Modes: normal, `--per-op-conn`, `--reconnect`. |
| `dsnode` | C | A node with an append-only record log, recovery, and a report region kept in a shared mapping. Options `--threads=2`, `--pause`. |
| `dsgo` | Go | The Go server of the ring (netpoller, one goroutine per connection). |
| `dsgonode` | Go | The Go version of `dsnode`. |
| `dsrs` | Rust (std only) | The Rust server of the ring (blocking sockets, one thread per connection). |

## Build and run natively

```
make              # build everything into build/ (needs a C11 compiler, Go 1.24+ and a Rust toolchain)
make c            # only the C programs
make test         # every native self-test (see below)
make clean        # remove build/ and rust/target
```

`make test RUNS=20 KILLS=1000 GO_KILLS=200` is the full run used on Linux CI. The defaults (`RUNS=20`, `KILLS=100`,
`GO_KILLS=40`) are quicker. `DS_PORT_BASE` (default 17100) moves every loopback port the tests use. The Go programs are
built with `CGO_ENABLED=0`, so they are static.

Run a ring trial by hand (ports and directories are arguments; nothing is a fixed global):

```
TRIAL=$(mktemp -d)
build/dsl --topo=c3e --bin-dir=build --tmp="$TRIAL" --port-base=7101
```

Topologies: `c3e` (C servers, epoll), `c3t` (timeout), `c3t2` (timeout with a heartbeat thread), `c3b` (blocking
sockets, one connection per operation), `g3` (Go servers), `r3` (Rust servers). Each starts one launcher, three servers
(ports `base`, `base+1`, `base+2`) and two clients.

Run a crash trial by hand:

```
TRIAL=$(mktemp -d)
build/dsl --crash=pid,2 --bin-dir=build --tmp="$TRIAL" --port=7201 --seed=620001
```

`--crash=pid,K` or `--crash=pgid,K` with K of 1 or 2; `--node-bin=dsgonode` for the Go node; `--threads=2` and
`--pause` are passed to the node; `--helper` (with `pgid`) makes the launcher itself start a helper process in the first
incarnation's process group; `--raw-write-kill` (with `--pause`, one kill) does an alive check, then a `write`
immediately followed by the kill, with no other system call between. `build/dsl --print-plan --seed=S --kills=K` prints
the crash plan and exits. The seed comes from `--seed` only; no environment variable is read.

## Output

Every line is written with one `write(2)`. No line contains a pid, a port, a path or a clock reading in clear (the only
clock-derived field is a digest, `clock=`, which is zero outside the timeout modes).

Ring trial (prefix `DSN0 v1`):

```
role=srv node=<n1|n2|n3> mode=<m> applied repl_in repl_out refused hb suspect clock=<hex16> state=<hex16>
role=cli node=<c1|c2> ops=40 ok fail refused hist=<hex16>
role=launch servers=3 clients=2 exits=<n1:s,...> stragglers live_after seq seq_ok hist=<hex16> ok
```

Crash trial (prefix `DSC0 v1`), plus `HAIRSPRING_EVENT` lines of kind `killed` (one per confirmed kill) and `exited` (one
per child process, with its raw wait status and a cause of `planned_kill`, `stop_kill`, `exit`, `signal` or `unreaped`):

```
role=node tag=<n1.iK> recovered torn                 (one per incarnation, at start-up)
role=node tag=<n1.iK> exit=quit done                 (the last incarnation, on QUIT)
role=sup kill victim pre_dead wifsig termsig reap_polls grp_gone region_seen planned_ms
role=cli ops=60 ok fail eof reset refused resent hist=<hex16>
role=launch kills restarts stragglers unexpected live_after seq_ok hist=<hex16> ok
```

The launcher exits 0 when every check passed and 3 otherwise (2 for a usage or set-up error). In `exits=`, a value is
the exit code, or `sig<N>` if the process was killed by a signal.

## How a trial works

- The launcher creates, under `--tmp`, `ds-seq` (4,096 zero bytes, mapped shared by the clients: the first 8 bytes are
  an atomic counter), `ds-hist/` (one history file per client) and, in crash mode, `ds-data/` (the node's log) and
  `hairspring-region/` (one report-region file per incarnation). Under an experiment these live in the trial's `/tmp`.
- Clients take one sequence number when an operation is invoked and one when it completes. The launcher sorts all
  history records by sequence number and checks they are exactly `1..N` with no gap or duplicate.
- The launcher never makes a blocking wait: it polls `waitpid(WNOHANG)` with short sleeps, bounded by a poll count. A
  process that is still alive at its bound is killed by pid and counted as a straggler. It reaps every child before it
  prints its line, and checks that no child is left.
- Servers print their result line when sent `QUIT`. They drain what is already delivered to their sockets first, so that
  in the C servers every replicated write is counted before the line is printed.
- The crash plan is a pure function of the seed: a splitmix64 finaliser keyed by a salt and the kill index, never by a
  global draw counter. Kill 1 lands at 40 to 280 ms, restart delays are 20 to 80 ms, and kill 2 follows 30 to 150 ms
  after the restart. The supervisor sleeps these intervals and reads no clock.
- The report region is 4,096 bytes: a 64-byte header (`HSRGNv1\0`, version 1, header size 64, the tag) and then three
  counters, `started`, `done` and `acked`, moved with release atomics and no system call. The node does
  `started`, one `write(2)` of a 32-byte record, `done`, sends `OK`, `acked`. A SIGKILL leaves the mapping in the file.

## Fixture rules

Checked by `tests/check_rules.py` (a grep over the sources, which first proves each rule fires on a bad sample) and by
the native tests:

- finite timeouts only: no `-1` to `epoll_wait` or `poll`, no `SO_RCVTIMEO`. The deliberate exceptions, where a call
  has no bound of its own, are: blocking `accept` and `recv` in `dskv --mode=block`; `pause()` in `dsnode --pause`;
  `TcpListener::incoming()` and the blocking reads in `dsrs`; and the blocking reads and accepts of the Go programs.
  The launcher bounds all of them by `QUIT` and then by SIGKILL, and every long-running program except `--pause` and
  `--mode=block` also ends itself after about 120 s without a request (`exit=idle`, status 4);
- numeric `127.0.0.1` only, no name lookups;
- no randomness (tables are arrays, never hash maps);
- every output line is one `write(2)`;
- the launcher never calls `unshare`, `setsid` or a namespace `clone`.

## Native tests

`make test` runs:

- `tests/check_rules.py`: the rule checks above.
- `tests/test_c.py`: the operation lists and the crash plan for 20 seeds against golden files and against the
  independent Python oracle in `tests/oracle.py`; the protocol of `dskv`, `dscli` and `dsnode` against scripted peers;
  recovery of a log with 0, 1 and 1,000 records, a torn tail and a bad record; `RUNS` runs of every ring topology and
  every crash topology; a server that exits early (`ok=0`, exit 3) and one that ignores `QUIT` (killed and counted); and,
  on Linux with `strace`, the adjacency of the raw write and kill and the `sched_yield` before an ordinary kill.
- `go vet`, `go test` and `tests/test_go.py`; `cargo test` and `tests/test_rust.py`.
- `tests/test_kills.py`: SIGKILL a serving node at pseudo-random points from a fixed seed and check the region
  invariant (`done <= records written <= started`, `started - done <= 1`, `acked <= done`, `done - acked <= 1`) and that
  the log has no torn entry.

Every test that starts processes checks afterwards that none of them, and no listener on its ports, is left. Set
`DS_REQUIRE_STRACE=1` to make a missing `strace` a failure instead of a skip.

Not run here: anything under Hermit, anything needing Linux x86-64 and Hermit.

## Things worth knowing

- `--pace-ms` on `dscli` sleeps after each operation. The crash trials pass 10 so that the planned kill lands while the
  client is still working; without pacing, 60 loopback operations finish in a few milliseconds natively.
- In the raw write-then-kill trial there is no client and no restart. Its `planned_ms` is the drawn plan, although the
  supervisor waits a fixed 50 ms before it acts.
- The `killed` line goes to stdout, with the other result lines.
- The history of a client that did not get a valid reply records `FAIL` as the result, and the client exits 3. The
  launcher then prints `ok=0` (ring trials: a non-zero exit status; crash trials: an unexpected `exited` line).
- The test hooks `--test-exit-early`, `--test-ignore-quit` (`dskv`), `--test-skip-seq` (`dscli`) and `--test-leak-child`
  (`dsl`), passed with `--server-arg=<node>:<arg>`, exist only so that the native tests can show a check failing. Two
  compile-time options exist for the same reason and are defined only by the tests: `DS_MUTANT_PID_KILL` (a group-kill
  trial that kills the pid alone) and `DS_STUB_WAITPID=1|2` (the first node's alive check errors, or the node is never
  reaped). The built programs never define them.
- A name that reaches a path (`--node`, `HAIRSPRING_PROC`) must be letters and digits (the tag exactly `<node>.i<digits>`),
  and a path or argument too long for the launcher's buffers is refused with status 2 rather than shortened.
- In a process-group trial the helper is a child of the launcher (not of the node), starts only with the first
  incarnation, joins its process group, and watches nothing: only the group kill removes it, so a kill that missed the
  group shows as `grp_gone=0` and an `exited` line with cause `stop_kill`. The launcher then reports `ok=0`.
- The launcher does an alive check just before every kill. A victim that is already gone gets `pre_dead=1`, no kill and no
  `killed` line; a victim that was not reaped after the kill gets no `killed` line either, and the plan stops.
- In the raw write-then-kill trial the launcher reports `ok=0` unless the node's region file existed at the kill
  (`region_seen=1`); its `planned_ms` is the fixed 50 ms it waits.
- A line that would not fit 4,096 bytes is not written truncated: the program writes nothing for it and exits 2.
