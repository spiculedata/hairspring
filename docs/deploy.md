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
  queue work for remote workers on suitable hosts (see [Remote workers](#5-remote-workers)).

## 2. Install

Install the `hairspring` command on the server and on every worker host. See [quickstart.md](quickstart.md#install) for
the options and [upgrading.md](upgrading.md#which-build-do-i-have) for the difference between the pre-release binary and
a build of `main`. Several options on this page exist only in builds newer than the v0.1-RC1 pre-release; each is marked
"newer than v0.1-RC1".

```bash
# From a checkout (needs Rust, a C compiler and linker; `make ui` also needs Node.js and npm):
make ui
bash scripts/install.sh --with-ui      # builds, installs to ~/.local/bin, then runs `hairspring doctor`

# Or a pre-release binary, checked against the release's SHA256SUMS:
bash scripts/install.sh --version v0.1-RC1

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
| The server | `--tokens-file FILE` | One `token:role` per line. Blank lines and lines starting with `#` are skipped. Keep the file at mode 600 |
| The server | `HAIRSPRING_TOKENS` | The same entries joined by commas: `token1:admin,token2:run,token3:read` |
| A worker | `HAIRSPRING_TOKEN`, or `--token-file FILE` | One **bare** token, no `:role`, nothing else in the file |

```bash
# The server's file: one token:role per line.
printf '%s:admin\n%s:run\n%s:read\n' "$ADMIN" "$RUN" "$READ" > /etc/hairspring/tokens
chmod 600 /etc/hairspring/tokens

# The worker's file: only the run token.
printf '%s\n' "$RUN" > /etc/hairspring/worker-token
chmod 600 /etc/hairspring/worker-token
```

A server tokens file with a mode that other users can read starts, with this warning on standard error:
`hairspring: warning: tokens is readable by other users (mode 644); run `chmod 600` on it`. A worker token file in
the wrong form is refused before it connects:
`error: multi: the worker token file must contain one bare token (the server's tokens file has token:role lines)`. A
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
- `--workers N` is the number of worker threads inside the server process (default 1). `--workers 0` means runs are
  executed only by remote workers.
- `--config` names the `hairspring.toml` that defines the targets (default `./hairspring.toml`; without one the
  built-in demo targets are offered). Callers of the API choose a target by name only; the commands behind a name
  come from this file, on whichever machine runs the job.
- `--db` is the SQLite results database (default `.hairspring/hairspring.db`, in the directory you start the server
  from).
- Newer than v0.1-RC1: `--organisation NAME` (shown in the web UI's top bar), `--max-connections`, `--max-replays` and
  `--trusted-proxy`; see [`hairspring serve --help`](#reference-serve-options) and [the reverse proxy](#9-reverse-proxy-and-tls)
  below.

At start the server prints where it is listening. Open the address in a browser for the web UI and sign in with a token.
Everything the server speaks is plain HTTP: put TLS in front of it before the address is reachable beyond one machine.

## 5. Remote workers

A worker pulls jobs from a server over HTTP, runs them with its own copy of the engine, and sends the results back.

```bash
export HAIRSPRING_TOKEN=...            # or: --token-file /etc/hairspring/worker-token
hairspring worker --server http://server.example:7878 --token-file /etc/hairspring/worker-token --name host-a
```

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

A remote worker starts **untrusted**: its results are shown, labelled as unverified, and cannot back a claim. An `admin`
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
warnings; it was not started, so treat the behaviour as not run here.

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
    --db /var/lib/hairspring/hairspring.db
Restart=on-failure
RestartSec=5
# Well above --max-connections (512 by default) plus 128.
LimitNOFILE=4096

[Install]
WantedBy=multi-user.target
```

`/etc/hairspring/tokens` must be readable by the `hairspring` user and by nobody else (mode 600). For PostgreSQL add
`EnvironmentFile=/etc/hairspring/server.env` with a `HAIRSPRING_DATABASE_URL=...` line in it, mode 600, and drop `--db`.

A unit for a worker (checked the same way):

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
    --server http://server.example:7878 \
    --token-file /etc/hairspring/worker-token \
    --config /etc/hairspring/hairspring.toml \
    --db /var/lib/hairspring-worker/hairspring.db \
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
sqlite3 .hairspring/hairspring.db ".backup 'hairspring-backup.db'"

# PostgreSQL (not run here):
pg_dump "$HAIRSPRING_DATABASE_URL" > hairspring-backup.sql
```

The SQLite command was run: it produced a database that `hairspring --db hairspring-backup.db results` reads back. Copy
the backup off the machine. Bundle files that `hairspring test` writes to `.hairspring/bundles` are separate files; copy
that directory too if you rely on them. Tokens and `hairspring.toml` are not in the database: back those up separately.

**Upgrades.** Back up first, stop every server and worker, upgrade them all, then start them. A newer build migrates the
database in place and an older build then refuses it. There is no downgrade: to go back, restore the backup. Details and
the exact error are in [upgrading.md](upgrading.md#the-schema-version-error).

**Retention.** Stored runs accumulate. `hairspring gc` (newer than v0.1-RC1) reports what it would remove and removes
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
caller behind a proxy shares one rate-limit allowance. With `--trusted-proxy ADDR[,ADDR...]` (newer than v0.1-RC1; also
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

So for the server itself (which runs no process targets when started with `--workers 0`) the advice is to run it
unprivileged. For a worker that runs `process` targets under Hermit, the unprivileged case has not been tested, and
whether to run it as root is your decision as the machine's owner. The example worker unit above uses an unprivileged
user as the checklist says; change `User=` knowing it is untested.

## 11. Security checklist

[SECURITY.md](../SECURITY.md#hardening-checklist-for-a-deployment) has the list. In short: generate tokens with
`hairspring token`, keep them out of command lines, terminate TLS in front of the server, keep PostgreSQL private, give
`read` tokens to anything that only analyses, keep `hairspring.toml` under review, raise the file-descriptor limit above
`--max-connections` plus 128, and name your reverse proxy with `--trusted-proxy`.

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
| `--tokens-file FILE` | `token:role` lines, an alternative to `HAIRSPRING_TOKENS` |
| `--max-connections N` | Open connections at once, 1 to 1000000 (default 512). Newer than v0.1-RC1 |
| `--max-replays N` | Replays at once, 1 to 64 (default 2). Newer than v0.1-RC1 |
| `--organisation NAME` | Name shown in the web UI's top bar (also `HAIRSPRING_ORGANISATION`). Newer than v0.1-RC1 |
| `--trusted-proxy ADDR[,ADDR...]` | Proxies whose `X-Forwarded-For` is read (also `HAIRSPRING_TRUSTED_PROXY`). Newer than v0.1-RC1 |
