# Operate a Hairspring server

This page is for the person who runs `hairspring serve` for other people: the machine, the tokens, remote workers, the
database, keeping it running, backing it up and upgrading it. It describes what the commands do. For the threat model and
the full list of limits read [SECURITY.md](../SECURITY.md); for versions and uninstalling see
[upgrading.md](upgrading.md).

How each command here was checked is stated where it matters. Commands marked "not run here" were written from the code
and the existing documentation, and have not been run by the author of this page.

Hairspring is experimental. Before you expose a server beyond one machine, read the [security checklist](#11-security-checklist).

## 1. Pick a machine

- A server that only runs the built-in simulation targets or `wasm` targets runs on macOS or Linux and needs nothing
  special.
- Runs of `process` targets need **Linux on x86-64 with hardware performance counters** and a separately installed
  Hermit. Not every machine can do this, and only one bare-metal host has been measured: see
  [Is my machine suitable?](guides/is-my-machine-suitable.md) and the [support matrix](support-matrix.md).
- The server and the machines that run process targets do not have to be the same machine: a server on any host can
  queue runs for remote workers on suitable hosts (see [Remote workers](#5-remote-workers)). One exception: a **replay**
  of a process-target bundle (`POST /api/replay`, the MCP replay tool, the UI's Replay button) runs on the server itself,
  using the server's `hairspring.toml`. If you want replays of process bundles, the server host must also qualify:
  Linux x86-64, Hermit installed, `HAIRSPRING_HERMIT` set in the server's environment, and the same loader and kernel as
  the recording.

## 2. Install

Install the `hairspring` command on the server and on every worker host. See [quickstart.md](quickstart.md#install) for
the options and [upgrading.md](upgrading.md#which-build-do-i-have) for the difference between the pre-release binary and
a build of `main`. The current pre-release is v0.1-RC2. Several options on this page are not in the older v0.1-RC1; each
is marked "not in v0.1-RC1", and v0.1-RC2 has all of them.

```bash
# From a checkout (needs Rust, a C compiler and linker; --with-ui also builds the web UI, which needs Node.js and npm.
# Leave --with-ui out for a server without the UI):
bash scripts/install.sh --with-ui      # builds, installs to ~/.local/bin, then runs `hairspring doctor`

# Or a pre-release binary, checked against the release's SHA256SUMS:
bash scripts/install.sh --version v0.1-RC2

hairspring --version
```

A binary built without the web UI still serves the API and MCP, and shows a placeholder page instead of the UI.

## 3. Create tokens

A server with any token configured requires one on every request. There are three roles, in increasing order of power:
`read` (look at results), `run` (start and cancel runs, replay bundles, and act as a worker) and `admin` (also change a
worker's trust). A token must be at least 16 characters; `hairspring token` makes 64-character hexadecimal ones.

```console
$ hairspring token --role admin
New API token (shown once, store it safely):
  <64 hexadecimal characters>
Give it to the server with:  HAIRSPRING_TOKENS=<token>:admin
Give it to a worker with:    HAIRSPRING_TOKEN=<token>
```

The command only prints a token. It does not store it: the server holds only a SHA-256 digest of each token it is given
at start, so keep the printed value somewhere safe (it is never shown again). `--role` defaults to `run`; an unknown role
gives `error: unknown role `bogus` (use read, run or admin)` and exit status 2. Run it once per token you need. A
typical set is one `admin` token for you, one `run` token for the workers, and a `read` token per dashboard or agent.

### The two token formats

The server and a worker read tokens in different formats. Mixing them up is the most common mistake.

| Who reads it | Where | Format |
|---|---|---|
| The server | `--tokens-file FILE` | One `token:role` per line, optionally `token:role label`. Blank lines and lines starting with `#` are skipped. Keep the file at mode 600 |
| The server | `HAIRSPRING_TOKENS` | The same entries joined by commas: `token1:admin,token2:run,token3:read`, or with labels `token1:admin alice,token2:run ci bot` |
| A worker | `HAIRSPRING_TOKEN`, or `--token-file FILE` | One **bare** token, no `:role`, nothing else in the file |

#### Token labels (optional)

A label is a short name for a token, so that a log line says who did something instead of showing an opaque key. Write
it after the role, separated by one space: `token:run ci bot`. Entries without a label work exactly as before.

- Only you can set a label, in this file or in `HAIRSPRING_TOKENS`. A caller cannot choose or change one.
- A label is 1 to 64 characters: ASCII letters and digits, space, `.`, `_` and `-`. No commas or colons (a comma ends
  the entry), no leading, trailing or doubled spaces, and no other characters, so a label cannot hold control or
  direction-changing characters. Exactly one space separates the label from the role; a tab, a second space or any
  other whitespace there is refused.
- Labels must differ from every other label, ignoring case, and a label must not contain any configured token. A
  label that does is refused at start. Errors read `entry N: ...` and do not repeat the token, the label or the role
  text. N counts entries, not file lines: blank and comment lines are not counted, and a line with commas counts as
  several entries.
- A label is not a secret and does not prove who is calling: two people who share a token share its label. Do not put
  a password, a token or personal data in one.
- The label appears in the server's log lines for creating a run, starting a replay, registering a worker and changing
  a worker's trust, as `label (t:<key>)`, never the token. `GET /api/me` returns it as `label` to the holder of that
  token only. It is not in any other response. Actions taken through MCP over HTTP are not yet logged with the label.
  The `t:<key>` is a prefix of the token's SHA-256 digest, so use tokens made by `hairspring token`.
- An older server refuses a labelled entry (it reads `run ci bot` as an unknown role), so it fails closed.
- Open mode (no tokens) has no labels.

Create the account the service runs as, and the files, so that the file is never readable by others and is owned by the
service user (as root; the commands were run in a Ubuntu 24.04 container, see [the systemd section](#7-keep-it-running-a-systemd-unit)):

```bash
useradd --system --home-dir /var/lib/hairspring --shell /usr/sbin/nologin hairspring
install -d -m 755 /etc/hairspring

# The server's file: one token:role per line. Create it empty with the right owner and mode, then write into it.
install -m 600 -o hairspring -g hairspring /dev/null /etc/hairspring/tokens
printf '%s:admin\n%s:run\n%s:read\n' "$ADMIN" "$RUN" "$READ" > /etc/hairspring/tokens

# The worker's file, on each worker host (run the useradd above there first): only the run token, same way.
install -m 600 -o hairspring -g hairspring /dev/null /etc/hairspring/worker-token
printf '%s\n' "$RUN" > /etc/hairspring/worker-token
```

The service user must own both files, or the service cannot read them. (Under a unit with `StateDirectory=`, systemd
creates the data directory for that user; the `--home-dir` above is only where the account's home would be.)

A server tokens file with a mode that other users can read starts, with this warning on standard error:
`hairspring: warning: tokens is readable by other users (mode 644); run `chmod 600` on it`. A worker token file in
the wrong form is refused before it connects:
`error: /etc/hairspring/tokens: the worker token file must contain one bare token (the server's tokens file has token:role lines)`
(the path is the one you gave). A
worker with no token, or with a `read` token, stops with `error: the server rejected this worker's token`. Do not pass
tokens on a command line, where other users can see them in the process list; use the file or the environment.

Tokens are read once, at start. To add, remove or change one, edit the file and restart the server (token rotation
without a restart is not implemented; see SECURITY.md).

## 4. Start the server

```bash
hairspring serve --addr 0.0.0.0:7878 --tokens-file /etc/hairspring/tokens --workers 0
```

- `--addr` defaults to `127.0.0.1:7878`. A non-loopback address without tokens is refused:
  `error: cannot start the server: refusing to listen on a non-loopback address without API tokens; configure at least one token`
  (exit status 2).
- With no tokens configured the server runs in "open mode": it accepts only loopback connections.
- `--workers N` is the number of worker threads inside the server process (default 1). `--workers 0` means queued runs
  are executed only by remote workers. It does **not** stop the server running target code: a replay
  (`POST /api/replay`, or the MCP replay tool; `run` role, at most `--max-replays` at once) runs the target on the server,
  using the server's `hairspring.toml`, whatever `--workers` says. So does a confirmation, but only when you turn it on
  with `--max-confirmations N` (default 0, off): then any `run` token can start up to N of them at once, each for up to
  two minutes, and the checks it stores are trusted. Leave it off unless every `run` token holder is someone you would
  let move a claim either way.
- `--config` names the `hairspring.toml` that defines the targets (default `./hairspring.toml`; without one the
  built-in demo targets are offered). Callers of the API choose a target by name only; the commands behind a name
  come from this file, on whichever machine runs the job.
- `--db` is the SQLite results database (default `.hairspring/hairspring.db`, in the directory you start the server
  from).
- Not in v0.1-RC1: `--organisation NAME` (shown in the web UI's top bar), `--max-connections`, `--max-replays` and
  `--trusted-proxy`; see [`hairspring serve --help`](#reference-serve-options) and [the reverse proxy](#9-reverse-proxy-and-tls)
  below. `--max-confirmations` is in neither v0.1-RC1 nor v0.1-RC2.

At start the server prints where it is listening. Open the address in a browser for the web UI and sign in with a token.
Everything the server speaks is plain HTTP: put TLS in front of it before the address is reachable beyond one machine.

## 5. Remote workers

A worker pulls jobs from a server over HTTP, runs them with its own copy of the engine, and sends the results back.

```bash
export HAIRSPRING_TOKEN=...            # or: --token-file /etc/hairspring/worker-token
hairspring worker --server https://hairspring.example.com --token-file /etc/hairspring/worker-token --name host-a
```

Use `https://`, through the TLS-terminating proxy described [below](#9-reverse-proxy-and-tls), so the token is not sent in
the clear. Given a plain `http://` address that is not loopback, the worker prints
`hairspring-worker: warning: the API token is sent over plain http to a non-local server; use https or a private network`
and carries on (this warning is from the code; it was not seen here, because the runs here used `127.0.0.1`).

Run against a local server, a worker with a valid `run` token prints
`hairspring: worker `host-a` serving http://127.0.0.1:7899` once the server has accepted its registration, and then
shows on the Workers page and in `GET /api/workers` as `"state":"idle"`, `"trusted":false`. When it cannot reach the
server it prints `hairspring-worker: registration failed, retrying: cannot reach the server: ...` and keeps trying.
`--poll-ms` (default 1000) is the wait between polls when the queue is empty.

The `capabilities` a worker reports about itself are not checked by the server. The worker currently always reports
`{"backends":["sim"]}`, even on a host that can run process targets, so do not use that field to decide whether a worker
has Hermit.

### What a worker host for `process` targets needs

A worker runs a job by looking the job's target name up in **its own** `hairspring.toml` and running the command it finds
there. So that host needs (Linux x86-64 parts: not run here):

- Its own `hairspring.toml`, passed with `--config`, that defines every target it should run, under the same names the
  server lists.
- The program under test at the **same absolute path** the target's `command` names, built the same way. A replay of a
  bundle recorded elsewhere is only reliable from the same paths, and for some programs (a JVM, for one) the paths change
  the schedule: see [determinism.md](determinism.md#recorded-environment).
- Hermit installed (`bash scripts/install-hermit.sh install`) and `HAIRSPRING_HERMIT` set to the `hermit` executable in
  the worker's environment.
- The same loader and kernel behaviour for replays. A bundle records hashes of the loader and libc, and a replay on a
  host where they differ is refused or reported as a changed environment. Treat hosts that run replays as a set of
  identical machines.
- A hardware performance counter (see [Pick a machine](#1-pick-a-machine)). Check with `hairspring doctor --process-backend`
  on that host.

### Trusting a remote worker

A remote worker starts **untrusted**: its results are shown, labelled as unverified, and cannot back a claim. Trusting a
worker changes only how its results are labelled and whether they can back a claim; it sends the worker no code. An `admin`
token marks a worker trusted with the API; there is no command-line subcommand for it. Find the worker's id with a `run`
or `admin` token (a `read` token is not shown ids), then post to `/api/workers/<id>/trust` with the JSON body
`{"trusted":true}`:

```console
$ curl -s -H "Authorization: Bearer $ADMIN" http://127.0.0.1:7878/api/workers
{"queue":{"queued":0,"running":0},"workers":[{... "id":"wrk_...","name":"host-a", ... "trusted":false}]}

$ curl -s -X POST -H "Authorization: Bearer $ADMIN" -H 'content-type: application/json' \
    -d '{"trusted":true}' http://127.0.0.1:7878/api/workers/wrk_.../trust
{"ok":true}
```

Run here against a local server: that gave `{"ok":true}` and the worker list then showed `"trusted":true`. `{"trusted":false}`
withdraws trust. A `run` token gets `403` (`this endpoint requires the admin role`), and a body without `trusted` gets `400`
(`missing field `trusted``). The worker's id is a credential for the worker endpoints; do not paste it into chat or tickets.
The trust state is shown on the Workers page ([quickstart](quickstart.md#the-workers-page)). A `run` token is a trust
domain: give it only to hosts you trust (SECURITY.md).

## 6. PostgreSQL

SQLite is the default and is enough for one server. For a team, or more than one server process, use PostgreSQL by
setting `HAIRSPRING_DATABASE_URL` in the server's environment (and in the environment of `hairspring gc`):

```bash
export HAIRSPRING_DATABASE_URL='postgres://hairspring:PASSWORD@db.internal/hairspring'
hairspring serve --addr 0.0.0.0:7878 --tokens-file /etc/hairspring/tokens --workers 0
```

- `hairspring serve --help` does not list this variable. It is read in `crates/hairspring-cli/src/ctx.rs`, and a `--db`
  value that starts with `postgres://` or `postgresql://` does the same. Prefer the environment variable: a password in
  `--db` is visible in the process list, and the command warns about it.
- `HAIRSPRING_DATABASE_SCHEMA` names the schema to use (default `public`).
- The URL is never printed or logged. A URL that cannot be reached gives
  `error: cannot use the PostgreSQL database: storage error: error connecting to server` (run here, against a closed port).
- The connection has no TLS. Keep PostgreSQL on a private network (SECURITY.md).
- Running against a live PostgreSQL was not done for this page (no PostgreSQL server was available): not run here.
  `docker-compose.yml` in the repository starts one for you; see [Docker Compose](#12-docker-compose).

Workers do not need the database: they talk to the server over HTTP.

## 7. Keep it running: a systemd unit

A unit for the server. `systemd-analyze verify` (systemd 255, in a container, with a stub executable) accepted it with no
warnings; it was not started, so treat the behaviour as not run here. The server unit uses systemd's sandboxing options
(`NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, and write access only to the data directory),
which keep the process from writing anywhere else. It uses `--workers 0`, so the server itself runs no queued jobs and
workers on other hosts do the work; leave `--workers 0` out to run jobs inside the server (the default is one thread).
A replay still runs target code on the server in either case, using the server's `hairspring.toml`. The server unit sets
no `HAIRSPRING_HERMIT`, and under `ProtectSystem=strict` and `ProtectHome=yes` a replay of a **process** bundle was not tested:
if you want such replays on the server, add `Environment=HAIRSPRING_HERMIT=...` and expect to loosen the hardening
(Hermit may need paths the options restrict). Simulation and `wasm` replays do not involve Hermit.

```ini
# /etc/systemd/system/hairspring.service
[Unit]
Description=Hairspring server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hairspring
Group=hairspring
StateDirectory=hairspring
WorkingDirectory=/var/lib/hairspring
ExecStart=/usr/local/bin/hairspring serve \
    --addr 127.0.0.1:7878 \
    --tokens-file /etc/hairspring/tokens \
    --config /etc/hairspring/hairspring.toml \
    --db /var/lib/hairspring/hairspring.db \
    --workers 0
Restart=on-failure
RestartSec=5
# Send SIGTERM to the server only, not to every process in the unit (see below).
KillMode=mixed
# Well above --max-connections (512 by default) plus 128.
LimitNOFILE=4096
# Sandboxing: the process may write only to its data directory.
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/hairspring

[Install]
WantedBy=multi-user.target
```

systemd stops the unit with SIGTERM, and `hairspring serve` treats SIGTERM like Ctrl-C: it stops accepting requests,
ends a confirmation that is running between checks, and exits. With the default `KillMode=control-group` systemd sends
that SIGTERM to every process in the unit at once, so the Hermit run in progress is killed too. The server does not judge a
run that ended with the stop: that check is stored skipped, as for Ctrl-C, so the confirmation is unfinished and the
finding stays where the last finished confirmation left it. This depends on the server hearing the stop within half a
second of the Hermit process dying; with the default mode both signals are sent together, which is well inside that,
but a stop that reaches the server later is not noticed and the killed run counts as a miss. The strict verify has no
such handling. With `KillMode=mixed`, as in the unit above, SIGTERM goes to the server alone: the run in progress
finishes (which takes up to the target's trial time): a matched rerun is stored inconclusive, a replay that reproduced is
stored pass, and checks that did not run are skipped; then the server exits, with no window to depend on. Any processes still left after that
get SIGKILL, so keep `TimeoutStopSec` at its default or raise it above your targets' trial timeout, or systemd will
send SIGKILL first and nothing is stored. The unit was edited for `KillMode=mixed` after `systemd-analyze verify` was
run on it, and was not run through systemd again; the behaviour of both modes is from systemd's documentation, and was
checked here only with a test that signals the server and a fake Hermit together.

`/etc/hairspring/tokens` must be owned by the `hairspring` user and readable by nobody else (mode 600), as created in
[the token section](#3-create-tokens). The server listens on loopback; the worker's `--server` address is the proxy's
public `https://` name, which forwards to that port ([reverse proxy](#9-reverse-proxy-and-tls)). A worker sends its results to the
server and does not use `--db` (the code does not open the database), so the unit omits it. For PostgreSQL add
`EnvironmentFile=/etc/hairspring/server.env` with a `HAIRSPRING_DATABASE_URL=...` line in it, mode 600, and drop `--db`.

A unit for a worker (checked the same way). It has **no** sandboxing options on purpose: for process targets, Hermit
traces the program under test and may need things those options take away (a writable temporary directory, other
paths, ptrace itself). Whether a sandboxed worker still runs process targets was not tested, so the worker unit is left
as the minimum. If you want to limit what a worker can touch, which is what [SECURITY.md](../SECURITY.md#hardening-checklist-for-a-deployment)
asks for ("give workers only the filesystem access their targets need"), start from this unit, add options one at a
time, and run `hairspring doctor --process-backend` and a real run after each:

```ini
# /etc/systemd/system/hairspring-worker.service
[Unit]
Description=Hairspring worker
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hairspring
Group=hairspring
StateDirectory=hairspring-worker
WorkingDirectory=/var/lib/hairspring-worker
# Only for workers that run process targets.
Environment=HAIRSPRING_HERMIT=/opt/hermit/bin/hermit
ExecStart=/usr/local/bin/hairspring worker \
    --server https://hairspring.example.com \
    --token-file /etc/hairspring/worker-token \
    --config /etc/hairspring/hairspring.toml \
    --name %H
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then `sudo systemctl daemon-reload && sudo systemctl enable --now hairspring` (not run here). Whether a worker for
process targets should run as `User=hairspring` or as root is not settled by the documentation: see
[Root or not?](#10-root-or-not-for-process-target-workers).

## 8. Logs, backups, upgrades and retention

**Logs.** Startup lines go to standard output; warnings and internal errors go to standard error. The server writes no
request log that this page knows of. Under systemd both streams reach the journal (`journalctl -u hairspring`; not run
here). Internal errors are logged on the server and reported to callers generically (SECURITY.md).

**Backups.** Everything the server knows is in the results database.

```bash
# SQLite: SQLite's online backup (made for a database that is in use; not tried here against a busy server).
(umask 077; sqlite3 .hairspring/hairspring.db ".backup 'hairspring-backup.db'")

# PostgreSQL (not run here). Keep the password out of the process list: put it in ~/.pgpass (mode 600) or a file named by
# PGPASSFILE, then pass no password on the command line.
(umask 077; pg_dump -h db.internal -U hairspring hairspring > hairspring-backup.sql)
```

The SQLite command was run: it produced a database that `hairspring --db hairspring-backup.db results` reads back. The
backup holds your results, so it is created readable by its owner only (`umask 077`); copy it off the machine the same way. Bundle files that `hairspring test` writes to `.hairspring/bundles` are separate files; copy
that directory too if you rely on them. Tokens and `hairspring.toml` are not in the database: back those up separately.

**Upgrades.** Back up first, stop every server and worker, upgrade them all, then start them. A newer build migrates the
database in place and an older build then refuses it. The CLI has no downgrade command: to go back, restore the backup. Details and
the exact error are in [upgrading.md](upgrading.md#the-schema-version-error).

**Retention.** Stored runs accumulate. `hairspring gc` (not in v0.1-RC1) reports what it would remove and removes
nothing until you repeat it with the arguments the report prints:

```bash
hairspring gc --older-than 30d
```

Always look at the report first, then follow its last line. The full description, options and limits are in
[quickstart.md](quickstart.md#remove-old-runs-hairspring-gc).

## 9. Reverse proxy and TLS

There is no built-in TLS. Put a TLS-terminating reverse proxy (nginx, Caddy, or a load balancer) in front of the server,
bind the server to loopback or a private address, and let only the proxy reach it. The nginx and Caddy configurations,
including the event-stream timeouts, are in SECURITY.md under
[Running behind a reverse proxy](../SECURITY.md#running-behind-a-reverse-proxy); they are not repeated here so that
there is one copy.

In short: without `--trusted-proxy` the server counts every request under the address it is connected from, so every
caller behind a proxy shares one rate-limit allowance. With `--trusted-proxy ADDR[,ADDR...]` (not in v0.1-RC1; also
`HAIRSPRING_TRUSTED_PROXY`) the server reads the right-most `X-Forwarded-For` entry that is not itself a listed proxy,
but only on connections that come from a listed proxy. Configure the proxy to **overwrite** `X-Forwarded-For` with the
address it saw, list only proxies you control, and keep the server's port unreachable except through them.

## 10. Root or not, for process-target workers

The documentation says two things that do not fit together, and this page does not choose between them:

- [quickstart.md](quickstart.md#what-you-need) says: "The measurements behind this project ran Hermit as root. Running it
  as an unprivileged user was not tested."
- [SECURITY.md](../SECURITY.md#known-limitations-read-these) says to run workers as an unprivileged user and protect
  `hairspring.toml`, because process targets execute its commands with the server's privileges; its hardening checklist
  says "Run the server and workers as unprivileged users". The provided Docker image runs as an unprivileged user
  (uid 10001), and it contains no Hermit.

The server runs target code in three ways: its own worker threads (`--workers`, default 1), replays (`POST /api/replay`
and the MCP replay tool, `run` role, at most `--max-replays` at once) and, only when `--max-confirmations` is above 0,
confirmations (`POST /api/failures/{id}/confirm`, `run` role), which use the server's `hairspring.toml`. So the
advice to run the server unprivileged applies in every case, including `--workers 0`.

For a worker that runs `process` targets under Hermit, the unprivileged case has not been tested, and whether to run it
as root is your decision as the machine's owner. Know the consequence: a worker run as root runs every command in its
`hairspring.toml` as root whenever anyone holding a `run` token queues a run of that target. The example worker unit above
uses an unprivileged user as the checklist says; change `User=` knowing it is untested.

## 11. Security checklist

[SECURITY.md](../SECURITY.md#hardening-checklist-for-a-deployment) has the list. In short: generate tokens with
`hairspring token`, keep them out of command lines, terminate TLS in front of the server, keep PostgreSQL private, give
`read` tokens to anything that only analyses, keep `hairspring.toml` under review, give workers only the filesystem access
their targets need (the worker unit above is deliberately not sandboxed, see [section 7](#7-keep-it-running-a-systemd-unit)),
raise the file-descriptor limit above `--max-connections` plus 128, give the proxy its own per-client connection and rate
limits, and name your reverse proxy with `--trusted-proxy`.

## 12. Docker Compose

`docker-compose.yml` starts PostgreSQL, one server and two workers:

```bash
cp .env.example .env          # then replace every value; tokens must be at least 16 characters
docker compose up --build -d
```

(Not run here: starting the stack was not done for this page.) What to know before you rely on it:

- The image contains the `hairspring` binary and the simulated Dapr sidecar only. It has **no Hermit and no
  `hairspring.toml`**, so out of the box the workers run only the built-in simulation targets. `process` targets need
  workers on Linux x86-64 hosts with Hermit, as in [Remote workers](#5-remote-workers).
- Remote workers start untrusted, so results from the compose workers are labelled unverified until an admin trusts each
  worker ([Trusting a remote worker](#trusting-a-remote-worker)).
- The server is published on `127.0.0.1` only. Put a TLS-terminating proxy in front before exposing it.
- Generate the token values with `openssl rand -hex 24`, as `.env.example` says. The image runs as an unprivileged user.
- `hairspring --version` inside an image built without the build arguments in the Dockerfile prints `(unknown)` for the
  commit.

## Reference: serve options

From `hairspring serve --help` on a build of `main`:

| Option | Meaning |
|---|---|
| `--addr ADDR` | Address to listen on (default `127.0.0.1:7878`). Non-loopback needs tokens |
| `--config FILE`, `--db PATH` | Target configuration, and the SQLite results database |
| `--workers N` | Worker threads inside this process; 0 means remote workers only (default 1) |
| `--rate-limit N` | Requests per minute per caller; 0 disables (default 1200) |
| `--register-limit N` | Workers one token may register per hour; 0 disables (default 100; counted in memory) |
| `--tokens-file FILE` | `token:role` or `token:role label` lines, an alternative to `HAIRSPRING_TOKENS` |
| `--max-connections N` | Open connections at once, 1 to 1000000 (default 512). Not in v0.1-RC1 |
| `--max-replays N` | Replays at once, 1 to 64 (default 2). Not in v0.1-RC1 |
| `--max-confirmations N` | Confirmations a `run` token may start from the web UI or `POST /api/failures/{id}/confirm`, at once, 0 to 4 (default 0, which turns it off). Each runs a failure's `process` target on this server for up to two minutes, and its checks are trusted like the server's own workers'; see [SECURITY.md](../SECURITY.md#known-limitations-read-these). Not in v0.1-RC1 or v0.1-RC2 |
| `--organisation NAME` | Name shown in the web UI's top bar (also `HAIRSPRING_ORGANISATION`). Not in v0.1-RC1 |
| `--trusted-proxy ADDR[,ADDR...]` | Proxies whose `X-Forwarded-For` is read (also `HAIRSPRING_TRUSTED_PROXY`). Not in v0.1-RC1 |
