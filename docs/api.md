# HTTP API reference

`hairspring serve` serves a REST API under `/api`, a Model Context Protocol endpoint at `/mcp` and the web UI at `/`.
The web UI is a client of this API, so anything it shows you can be read by a script. This page is the reference for the
REST endpoints: who may call each one, what comes back, and how it fails. The MCP tools are listed in the
[README](../README.md#for-ai-agents-mcp). What the server does behind the endpoints (jobs, workers, storage) is in
[architecture.md](architecture.md); what it defends against is in [SECURITY.md](../SECURITY.md).

Every example below was run against a scratch `hairspring serve` with scratch tokens. Where a reply is long, the
example keeps the fields it is about and says so. JSON object keys come back in alphabetical order, as in the examples.

```bash
hairspring serve --addr 127.0.0.1:7878 --tokens-file tokens   # one `token:role` line each; tokens are 16+ characters, no whitespace
export TOKEN=...                                               # one of those tokens
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7878/api/me
```

## Authentication and roles

Every `/api/` endpoint except `GET /api/health` needs `Authorization: Bearer <token>`. A token has one role, and each
role includes the ones before it:

| Role | May |
|---|---|
| `read` | look at targets, runs, failures, trials, properties, workers and bundles; search; follow a run's events |
| `run` | also start, cancel and replay runs and sweeps, and act as a worker (`/api/worker/...`) |
| `admin` | also change a worker's trust |

Create a token with `hairspring token --role read|run|admin`; it prints the line to give the server
(`HAIRSPRING_TOKENS=token:role`, or one `token:role` per line in `--tokens-file`) and, for a worker, the bare
token (`HAIRSPRING_TOKEN`). A token must be at least 16 characters with no whitespace; `serve` refuses a shorter one,
and `hairspring token` generates valid ones. `serve` warns when the tokens file is readable by other users (mode 644);
run `chmod 600` on it. A server with no tokens is in **open mode**: it listens only on a loopback address, accepts
only same-origin local requests (anything else is a 403), and treats every caller as `admin`.

A missing or wrong token is a `401` with a `WWW-Authenticate: Bearer` header; a token whose role is too low is a `403`
that names the role:

```text
$ curl -s -i http://127.0.0.1:7878/api/me
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
...
{"error":"a valid bearer token is required"}

$ curl -s -X POST -H "Authorization: Bearer $READ_TOKEN" -H 'content-type: application/json' \
    -d '{"target":"pubsub-buggy"}' http://127.0.0.1:7878/api/runs
{"error":"this endpoint requires the run role"}
```

### Endpoints and the role each needs

Roles are checked in the handler, from the router in `crates/hairspring-server/src/lib.rs`.

| Method and path | Role | What it does |
|---|---|---|
| `GET /api/health` | none | `{"status":"ok","version":"0.1.0"}`; adds `organisation` for a caller with a valid token (see below) |
| `GET /api/me` | any valid token | who the server takes you to be, and what you may do |
| `GET /api/targets` | `read` | the targets the server can run (its configuration and the built-in ones), with their budgets |
| `GET /api/runs` | `read` | runs, newest first, with [paging and filters](#paging-and-filters) |
| `POST /api/runs` | `run` | start a search or a sweep |
| `GET /api/runs/{id}` | `read` | one run, with its failures |
| `POST /api/runs/{id}/cancel` | `run` | cancel a queued or running run |
| `GET /api/runs/{id}/events` | `read` | the run's events as a server-sent event stream |
| `GET /api/runs/{id}/properties` | `read` | the property table (parameters `status`, `kind`, `q`, `sort`, `cursor`, `limit`) |
| `GET /api/runs/{id}/properties/{idx}` | `read` | one property, with a page of seeds |
| `GET /api/runs/{id}/trials` | `read` | judged trials (parameters `verdict`, `fails`, `hits`, `misses`, `degraded`, `perturbing`, `fp`, `seed`, `cursor`, `limit`) |
| `GET /api/runs/{id}/trials/{seq}` | `read` | one trial |
| `GET /api/runs/{id}/trials/{seq}/output/{stream}` | `read` | a trial's kept output as a plain-text download |
| `GET /api/runs/{id}/strip` | `read` | the seed strip: one code per trial |
| `GET /api/runs/{id}/coverage` | `read` | cumulative coverage over trials |
| `GET /api/search?q=...` | `read` | [find an id or a name](#get-apisearchq) |
| `GET /api/failures` | `read` | failures, newest first, with [paging and filters](#paging-and-filters) |
| `GET /api/failures/{id}` | `read` | one failure: its steps and bundle |
| `GET /api/failures/{id}/checks` | `read` | the failure's stored confirmation checks and where they leave it |
| `GET /api/failures/{id}/confirm` | `read` | whether a [confirmation started from the API](#post-apifailuresidconfirm) is running |
| `POST /api/failures/{id}/confirm` | `run` | [confirm a failure on the server](#post-apifailuresidconfirm); off unless the server was started with `--max-confirmations` |
| `GET /api/bundles/{hash}` | `read` | a stored bundle as text |
| `POST /api/replay` | `run` | [replay a bundle](#post-apireplay) |
| `GET /api/workers` | `read` (worker ids from `run`) | [the workers and the queue](#get-apiworkers) |
| `POST /api/workers/{id}/trust` | `admin` | [set whether a worker is trusted](#post-apiworkersidtrust) |
| `POST /api/worker/register` | `run` | a remote worker registers; the reply holds its secret |
| `POST /api/worker/lease`, `/api/worker/jobs/{job}/progress`, `/complete`, `/fail`, `/trials`, `/inputs`, `/heartbeat` | `run`, plus the worker's secret | the worker protocol; used by `hairspring worker`, not by hand (see [architecture.md](architecture.md#job-lifecycle)) |
| `POST /mcp` | any valid token | one JSON-RPC message per request; each tool checks its own role (`start_run`, `replay_bundle` and `cancel_run` need `run`, the others `read`) |

The role is checked inside each handler, after the body of a `POST` has been read. A caller whose role is too low that
sends a malformed body to `POST /api/runs` or `POST /api/worker/register` therefore gets a 400 before it would get the
403 (checked: a `read` token sending `{"seed":1}` to either gets a 400 naming the missing field). A token that is
missing or wrong is refused with 401 before any body is read.

A request for an id the server does not know, or whose text is not an id, is a `404` (`{"error":"not found"}`).

### `GET /api/health`

```text
$ curl -s http://127.0.0.1:7878/api/health
{"status":"ok","version":"0.1.0"}
```

With `hairspring serve --organisation "Acme Ltd"` (or `HAIRSPRING_ORGANISATION`) the reply has an `organisation` field
for a caller with a valid token, and for anyone when the server has no tokens. A health request that carries a token is
counted against the rate limit like other requests; a request without an `Authorization` header is never limited and
never refused. The rules are in [architecture.md](architecture.md#job-lifecycle) ("Requests to `/api/`") and
[SECURITY.md](../SECURITY.md).

## `GET /api/me`

`GET /api/me` tells a caller who the server takes it to be and what it may do. It needs a valid token like the other
read endpoints, so a missing or wrong token gets the same 401. A real reply for a `run` token:

```json
{"can":{"admin":false,"confirm":false,"read":true,"run":true},"open_mode":false,"role":"run","roles":["read","run","admin"]}
```

- `role` is the caller's role: `read`, `run` or `admin`. `can` spells out what that role allows, and `roles` lists
  the role names in order.
- `can.confirm` is `true` only when the server was started with `--max-confirmations` above 0 **and** the caller's
  role is `run` or `admin`; otherwise it is `false`. A server that predates the setting has no `can.confirm` key,
  which a client should read as `false`. When it is `true` the reply also has `confirm`, with `wall_secs` (the
  budget of one confirmation, in seconds) and `max_running` (how many the server runs at once), for example
  `"confirm":{"max_running":2,"wall_secs":120}`. A server with the setting off says nothing about limits.
- `open_mode` is `true` when the server has no API tokens and only accepts same-origin local requests. Every caller is
  then treated as `admin`.
- `label` is present only when the operator gave the caller's own token a label in the token configuration (see
  [deploy.md](deploy.md#token-labels-optional)). It is the caller's own label and no one else's, it is not a secret and
  not proof of identity, and a caller cannot set it. A token without a label gets the reply above, with no `label`
  key. In open mode there is none.
- The reply never contains the token, a digest of it, or any other secret.

The web UI asks this when it loads, to disable the Start run button for a `read` token and to show the Run
confirmation control only when `can.confirm` is `true`.

## `POST /api/failures/{id}/confirm`

Starts a [confirmation](determinism.md#confirming-a-failure) of a stored failure on the server and answers at once;
the work goes on in the background. It reruns the failure's seed, replays its bundle and records every check through
the same path as `hairspring confirm`, so the ladder, the claim gate and the trust label are the same. The caller
sends no body and chooses nothing but the failure: the reruns and the wall-clock budget are the defaults of
`hairspring confirm` (3 reruns, 120 seconds), and there is no control target and no environment override.

- **Off by default.** The server must have been started with `hairspring serve --max-confirmations N`, N from 1 to 4
  (see [deploy.md](deploy.md#reference-serve-options)). With the default of 0 the route answers `403` with
  `confirmation from the browser is not enabled on this server` and `"code":"confirm_disabled"`, whatever the id.
- **Role.** `run` or `admin`. A `read` token gets `403` (`this endpoint requires the run role`), checked first.
- **The failure's bundle must be for the failure's target.** A confirmation reruns the bundle's own target. Every way
  of confirming a stored failure (this route, `hairspring confirm <failure id>`, `hairspring confirm <run id>` and
  `hairspring test --confirm`) refuses one whose stored bundle names a different target or cannot be read, before
  anything runs, and this route answers `400`. The server also refuses to store such a failure when a worker reports
  one. So a confirmation cannot record checks against a target they did not run, even for data stored by an older
  version.
- **It runs on the server's own engine, never on a remote worker.** The server needs the failure's target in its own
  `hairspring.toml`, and only `process` targets can be confirmed, so on a server that cannot run them (no Hermit, a
  host that is not Linux on x86-64) a confirmation is stored with every check `inconclusive` and says Hermit could not
  be started, exactly as `hairspring confirm` does there; it does not hang. A confirmation whose every check is inconclusive (for example because Hermit could not be started) still counts as finished. A failure's finding is read from the newest finished confirmation, so such a confirmation replaces the finding from the one before it and can move a confirmed failure back to candidate. The earlier checks stay stored and listed.
- **Its checks are trusted.** The server is the runner, as for its own workers, so what a confirmation stores can
  move a failure's state, and a claim, in either direction. Any `run` token can start one while the setting is on.
- **At most one at a time per failure, and at most N at once.** A second request for a failure that already has
  one running is `409`; a request when all N are running is `429` with `Retry-After`. Nothing is queued. A
  confirmation keeps its slot until it ends, even if the client goes away. Stopping `hairspring serve` with Ctrl-C
  or SIGTERM (what systemd sends) ends a confirmation between checks. The run in progress finishes. If it was a rerun
  that matched, that check is stored inconclusive (stopped: cancelled) with the runs that did match; if it was a replay
  that reproduced, it is stored pass, and the confirmation counts as finished (that is correct evidence); if it did not
  match or reproduce (for example the stop also killed the Hermit process, as systemd's default
  `KillMode=control-group` does), that check is stored skipped, as `hairspring confirm` stores it on Ctrl-C. Checks
  that did not run are skipped, and the server exits. A confirmation with a
  skipped check is unfinished, so the finding is still read from the newest confirmation that finished and a cut run
  does not change it. This holds when the server hears the stop within half a second of the Hermit process ending
  (it waits that long before treating a run killed by SIGTERM, or a replay that did not reproduce, as a miss); a stop
  that reaches the server later than that is not noticed, and such a run counts as a miss. The strict verify has no
  such handling. See [deploy.md](deploy.md#7-keep-it-running-a-systemd-unit) for `KillMode=mixed`, which signals the
  server alone and so avoids the window.
- The server logs one line when it starts and one when it ends, naming the failure id and the caller as
  `label (t:<key>)` (or the key alone), never the token.

A real reply, for an accepted request:

```json
{"accepted":true}
```

with status `202`. Read the result from `GET /api/failures/{id}/checks`.

| Status | When | Message or note |
|---|---|---|
| 202 | accepted | `{"accepted":true}` |
| 400 | the failure's target is not configured on this server, or is not a `process` target, or its stored bundle is for another target or cannot be read | `this server has no configuration for the failure's target, so it cannot confirm it`; `only a failure of a process target can be confirmed, and this one is not`; `the failure's bundle is for a different target, so it cannot be confirmed`; `the failure's bundle cannot be read, so it cannot be confirmed` (no target name is repeated) |
| 401 | no token, or a wrong one | `a valid bearer token is required` |
| 403 | role too low; or the setting is off | `this endpoint requires the run role`; `confirmation from the browser is not enabled on this server` with `"code":"confirm_disabled"` |
| 404 | an id that is not a failure id, or a failure the server does not know | `not found` |
| 409 | this failure already has a confirmation running | `a confirmation of this failure is already running` with `"code":"confirm_running"` |
| 429 | N confirmations are already running | `too many confirmations are running; try again shortly`, `Retry-After: 30` |

In open mode (no tokens) the usual `Host` and cross-site `Origin` checks apply to this route like any other.

### `GET /api/failures/{id}/confirm`

The `read` role is enough. It says whether a confirmation started from the API is running for the failure, as this
server process knows it:

```json
{"enabled":true,"state":"running"}
```

`state` is `idle`, `running` or `failed`. `failed` means the last attempt ended in an error (the reason is in the
server's log) and carries `reason`, a fixed sentence. The state is kept in memory, so a restart forgets it; the
stored checks, which are what counts, come from `GET /api/failures/{id}/checks`. `enabled` is `false` when the server
was started without `--max-confirmations`. An id that is not a failure id, or an unknown failure, is `404`.

## `GET /api/workers`

`GET /api/workers` lists the workers the server knows and the queue behind them. The `read` role is enough. The list
is not paged: the server holds at most 1000 workers (see [architecture.md](architecture.md)), and a worker that has not
been heard from for 24 hours is removed when the table is full. A real reply from a server with its own worker, for a
`run` token:

```json
{"queue":{"queued":0,"running":0},"workers":[{"capabilities":{"backends":["embedded"]},"caps_json":"{\"backends\":[\"embedded\"]}","current_jobs":[],"id":"wrk_280cd2a30e1c3042","last_seen":1793907940991,"leased_jobs":0,"name":"embedded-0","registered_at":1793907915446,"state":"idle","trusted":true}]}
```

and the shape of a worker that holds a job (a trimmed example, not a captured reply):

```json
{
  "workers": [
    {
      "id": "wrk_…",
      "name": "remote-1",
      "caps_json": "{\"backends\":[\"sim\"]}",
      "registered_at": 1730000000000,
      "last_seen": 1730000060000,
      "trusted": false,
      "state": "busy",
      "capabilities": { "backends": ["sim"] },
      "current_jobs": [
        { "run_id": "run_…", "kind": "run", "lease_until": 1730000180000, "attempts": 1 }
      ],
      "leased_jobs": 1
    }
  ],
  "queue": { "queued": 1, "running": 1 }
}
```

- `state` is `busy` when the worker holds a lease that has not ended, `idle` when it holds none and the server heard
  from it within one lease length (`lease_secs`, 120 seconds by default), and `not_seen_recently` otherwise. It is
  worked out when you ask; nothing is stored.
- `trusted` is the flag an administrator sets ([`POST /api/workers/{id}/trust`](#post-apiworkersidtrust)). Remote workers
  start untrusted; the server's own workers are trusted.
- `capabilities` is what the worker sent when it registered, when that was valid JSON, and `null` otherwise. The
  original text is `caps_json`. Capabilities are the worker's own statement and are not checked.
- `current_jobs` names up to three jobs the worker holds, oldest first: the run, the kind of job, when the lease ends
  and how many times the job has been leased. `leased_jobs` counts the live leases the server reads at once, which is at
  most 5000 in all, so a fleet holding more than that is undercounted.
- `queue.queued` is the number of jobs waiting for a worker and `queue.running` the number held under a live lease.
- `id` is left out unless the token has the `run` role or more, because a worker id is a credential for the worker
  endpoints. The same call with a `read` token returns the same worker without `id`. The reply never contains a worker
  secret or its digest, a job payload, an error text or guest output.
- With `Accept: application/x-ndjson` each worker is one line and `queue` is on the closing control line under `page`
  ([ndjson.md](ndjson.md)).

## `POST /api/workers/{id}/trust`

Needs the `admin` role. Claims need trusted workers; data from an untrusted worker is shown, labelled as unverified
(see [architecture.md](architecture.md#job-lifecycle)). The body is JSON
with one field, and the id is the worker's `id` from `GET /api/workers` (which needs a `run` token to see it):

```text
$ curl -s -X POST -H "Authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
    -d '{"trusted":true}' http://127.0.0.1:7878/api/workers/wrk_64a2c08046c7c403/trust
{"ok":true}
```

A `run` token gets `{"error":"this endpoint requires the admin role"}` (403). Send `{"trusted":false}` to withdraw
trust. The Workers page of the web UI shows trust and does not change it.

## `GET /api/search?q=...`

`GET /api/search?q=run_2` looks for ids and names that start with, or equal, the text you give. It needs a valid token
(any role), is counted by the rate limit like the other read endpoints, and returns grouped hits. A real reply
(`q=run_2` on a server holding two runs):

```json
{"failures":[],"more":[],"properties":[],"runs":[{"id":"run_26aae667a02bbe2c","state":"failed","target":"pubsub-buggy"}],"schedules":[],"scope":{"per_group":5,"property_runs":2,"recent_runs":1000},"seeds":[],"targets":[]}
```

and `q=pubsub` on the same server:

```json
{"failures":[],"more":[],"properties":[],"runs":[],"schedules":[],"scope":{"per_group":5,"property_runs":2,"recent_runs":1000},"seeds":[],"targets":[{"name":"pubsub-atomic"},{"name":"pubsub-buggy"}]}
```

- `q` is 1 to 64 characters from ASCII letters, digits and `_ - . : /`. Anything else, a missing `q` and an empty one
  are a 400 (`{"error":"`q` is not valid"}`) that never repeats what was sent. `%` and `_` are ordinary characters,
  not wildcards, and matching is case-sensitive.
- What is searched: run ids and failure ids (a whole id or its start), target names (the start of a name), property
  ids (the start of an id), seeds (digits, the whole seed) and schedule fingerprints (16 to 64 lower case hex digits,
  the whole fingerprint). It matches ids and names only. It never reads guest output, kept output, bundles or failure
  messages, and it returns none of them.
- Each group has at most 5 hits. `more` lists the groups that have further hits, so a search box can say "more"
  instead of pretending it showed everything. The order differs by group:
  - `runs` and `failures`: newest first. A whole id of a run or failure older than the window comes after the others.
  - `targets`: in name order.
  - `properties`: each id once, with the newest run whose catalogue holds it; newest run first, then in id order
    (bytewise, so upper case before lower case) within a run. The id is untrusted text from the program under test,
    so it sits under a `guest` object, cut to one line.
  - `seeds`: first the runs started with that seed (`trial` is null), newest first, then the trials that ran it,
    newest run first and then by trial number.
  - `schedules`: newest run first, then by trial number.
- **`scope`** says how far the search looked. It looks at recent work: the start of a run id, a failure id, a seed or
  a schedule fingerprint is looked for in the `recent_runs` newest runs (1000), and a property id in the newest
  `property_runs` of them (10 at most). `scope.property_runs` is the number of runs whose catalogues this search
  read: 10, or fewer when fewer runs are stored, and 0 when the text is a whole run id, a whole failure id, only
  digits, or 16 to 64 lower case hex digits, which are not looked for as property ids. `per_group` is the number of
  hits each group holds at most. A whole run id or failure id is also looked up directly, so one of any age is found.
  An empty answer therefore means "none in the recent runs", not "never stored". To go further back use the filters on
  `GET /api/runs` and `GET /api/failures`.
- `q` given more than once: the last one is used, as with the other query parameters.

The web UI's top-bar search box uses this endpoint. What each group costs a server is in the "Search" section of
[architecture.md](architecture.md#search).

## Paging and filters

`GET /api/runs` and `GET /api/failures` need the `read` role and list newest first.

| Parameter | Meaning |
|---|---|
| `limit` | Rows per page. Default 50, at most 500; a larger value is lowered to 500 and 0 is raised to 1. Not a whole number: 400 |
| `cursor` | The `next_cursor` of the previous page: a position in the filtered list, a plain number from 0 to 100000. Anything else, or a larger number: 400 |

The reply is `{"runs": [...]}` or `{"failures": [...]}` with `"next_cursor": "<position>"` added when more rows follow;
the last page has no `next_cursor`. A real first page of one run:

```text
$ curl -s -H "Authorization: Bearer $TOKEN" 'http://127.0.0.1:7878/api/runs?limit=1'
{"next_cursor":"1","runs":[{"backend":"sim","build_id":null,"catalog_size":0,"config_json":"{...}","created_at":1793907928428,"distinct_schedules":0,"error":null,"finished_at":1793907929615,"id":"run_0f414ed707f5e05c","inconclusive":0,"ingest_attempt":1,"ingest_incomplete":false,"mode":"sweep","seed":1,"started_at":1793907928522,"state":"passed","steps":200000,"stopped_at_first_failure":false,"target":"pubsub-atomic","trials":2,"work":0}]}
```

(`config_json` is shortened here; it holds the run's request.) Filters are optional and all must match. A value that is
not valid is a 400 whose message names the parameter and never repeats the value:

```text
$ curl -s -H "Authorization: Bearer $TOKEN" 'http://127.0.0.1:7878/api/runs?state=bogus'
{"error":"`state` is not valid"}
```

| Endpoint | Parameter | Meaning |
|---|---|---|
| `/api/runs` | `state` | `queued`, `running`, `passed`, `failed`, `error` or `cancelled` |
| `/api/runs` | `target` | the exact target name (1 to 64 bytes, no control characters) |
| `/api/runs` | `mode` | the mode the run was stored with: `search` or `sweep` |
| `/api/failures` | `run` | a run id: only that run's failures |
| `/api/failures` | `property` | a property id prefix (1 to 128 bytes), matched against runs in which a property with such an id is stored as a violation |
| `/api/failures` | `status` | `candidate`, `rerun_repeated`, `confirmed`, `verified`, `not_reproducible` or `control_failed` |

A `status` filter makes the server read the stored checks of the failures it passes over, so one request reads at most
1000 failures and a page can come back short, or empty, with a `next_cursor` to carry on from: keep following
`next_cursor` until it is absent. A cursor is a position, so a run stored while a client pages moves later rows down
by one; drop ids you already have. The costs, measurements and exact rules are in
[architecture.md](architecture.md#paging-and-filters-on-the-runs-and-failures-lists). There is no "mine" filter: a run
does not record who started it.

### NDJSON

Every list can be read as one JSON object per line with `Accept: application/x-ndjson`. The last line is always a
control line, `{"_next":"<cursor>"}` when there is more and `{"_next":null}` when there is not. Real output:

```text
$ curl -s -H "Authorization: Bearer $TOKEN" -H 'Accept: application/x-ndjson' 'http://127.0.0.1:7878/api/runs?limit=1'
{"backend":"sim","build_id":null,...,"work":0}
{"_next":"1"}
```

The lines, the escaping, the lists that support it and the command-line form are in [ndjson.md](ndjson.md).

## `POST /api/runs`

Needs the `run` role. Starts a run and answers `201 Created` with the run, queued. The body is JSON:

| Field | Meaning |
|---|---|
| `target` | required: a target name |
| `seed` | optional whole number; without it the server picks one |
| `mode` | optional: `search` (the default; stops at the first confirmed failure) or `sweep` (runs the whole trial budget, continues after a failure and stores every trial) |
| `budget` | optional object with `max_steps`, `max_trials` and `wall_secs`. Values are clamped to the server's maxima (50,000,000 steps, 10,000 trials, 600 seconds); an unknown key is a 400 |

```text
$ curl -s -X POST -H "Authorization: Bearer $RUN_TOKEN" -H 'content-type: application/json' \
    -d '{"target":"pubsub-buggy","seed":5}' http://127.0.0.1:7878/api/runs
{"run":{"backend":"sim",...,"id":"run_26aae667a02bbe2c",...,"mode":"search","seed":5,"state":"queued","target":"pubsub-buggy",...}}
```

Poll `GET /api/runs/{id}` (or follow the events) until `state` is `passed`, `failed`, `error` or `cancelled`. A body that
is not valid is a 400, for example `{"error":"invalid request body: ... missing field `target` ..."}` or
`{"error":"unknown target `nope`"}`. `POST /api/runs/{id}/cancel` (also `run`) answers `{"cancelled":true}` when it
stopped a queued or running run and `{"cancelled":false}` when the run had already ended.

## `GET /api/runs/{id}/events`

A server-sent event stream (`Content-Type: text/event-stream`) of the run's events: the stored events first, then new
ones until the run ends. `?after=N` resumes after event id `N`. A real stream for a finished run:

```text
id: 1
event: started
data: {}

id: 2
event: progress
data: {"message":"violation found","steps":252,"trials":1}

id: 3
event: finished
data: {"distinct_schedules":0,"inconclusive":0,"state":"failed","work":0}
```

A stream ends when its run ends, when the server stops, or after 10 minutes, and the server holds at most 64 open
streams (16 per caller). For a `read` caller the `worker` key is removed from event payloads, as worker ids are left
out of `GET /api/workers`. A bad query string is a 400 (`{"error":"invalid query string"}`).

## `POST /api/replay`

Needs the `run` role, because a replay runs the target's code. The body holds the bundle text, and optionally
`allow_environment_change` (default false), which replays even when the host's dynamic loader differs from the
recording; the differences are still reported.

```text
$ curl -s -X POST -H "Authorization: Bearer $RUN_TOKEN" -H 'content-type: application/json' \
    -d @replay.json http://127.0.0.1:7878/api/replay     # replay.json is {"bundle": "<the bundle text>"}
{"reproduced":true,"steps":[...]}
```

`reproduced` is true when the property is still violated. A replay refused because the loader changed is a `409` with
`"code":"environment_mismatch"`. Only a few replays run at once, and one over the cap is refused with a 429 (below).
The command-line form is `hairspring replay`, with its own exit codes, in the [quickstart](quickstart.md#replay-a-bundle).

## Limits and errors

Every error is JSON, `{"error": "<one short line>"}`, and a few add `"code"`. The text never repeats what the caller
sent beyond a short, single-line, sanitised piece. A `429` and the `503` carry a `Retry-After` header.

| Status | When | Message or note |
|---|---|---|
| 400 | a body, query string, id or parameter that is not valid | names the parameter, for example `` `state` is not valid ``; `invalid query string` |
| 401 | no token, or a wrong one | `a valid bearer token is required`, with `WWW-Authenticate: Bearer` |
| 403 | role too low; or open mode and the request is not same-origin and local; or confirmation is off | `this endpoint requires the run role` (or `admin`); `"code":"confirm_disabled"` |
| 404 | an id the server does not know, or text that is not an id; also a run removed by `hairspring gc` | `not found` |
| 408 | the request body did not arrive within 30 seconds of its head | JSON error, connection closed |
| 409 | a replay refused because the loader differs from the recording | `"code":"environment_mismatch"` |
| 409 | a confirmation of the failure is already running | `"code":"confirm_running"` |
| 413 | a body over 2 MiB | |
| 415 | `/mcp` called without `Content-Type: application/json` | `Content-Type must be application/json` |
| 429 | the per-client rate limit (default 1200 requests a minute) | `rate limit exceeded`, `Retry-After: 60` |
| 429 | too many replays at once (default 2, `--max-replays`) | `too many replays are running; try again shortly`, `Retry-After: 5` |
| 429 | too many confirmations at once (`--max-confirmations`) | `too many confirmations are running; try again shortly`, `Retry-After: 30` |
| 429 | too many open event streams | `too many open event streams` or `too many open event streams for this caller` |
| 429 | a token registered too many workers in an hour (default 100, `--register-limit`) | `worker registration limit reached for this token`, `Retry-After: 3600` |
| 500 | an internal error | `internal error`; the detail is logged by the server, never returned |
| 503 | more open connections than `--max-connections` (default 512) | JSON error with `Retry-After: 1`, then the connection closes |

A real rate-limit refusal, from a scratch server started with `--rate-limit 3` (the fourth request in a minute):

```text
HTTP/1.1 429 Too Many Requests
retry-after: 60
...
{"error":"rate limit exceeded"}
```

`GET /api/health` without an `Authorization` header is never limited. Valid tokens and unauthenticated clients are
counted separately, and the client address is the connecting one unless the connection comes from a proxy named with
`--trusted-proxy`. How the limit, the connection cap, the time limits and the replay cap work, and what they do not
protect against, is written once, in:

- [architecture.md: Connection and request limits](architecture.md#connection-and-request-limits) (503, 408, 413, the
  replay and event-stream caps), and the paragraph on per-caller limits in [Job lifecycle](architecture.md#job-lifecycle)
  (429 and `--rate-limit`, `--register-limit`);
- [architecture.md: Client address behind a proxy](architecture.md#client-address-behind-a-proxy);
- [SECURITY.md](../SECURITY.md): known limitations, the reverse-proxy section and the deployment checklist.

API and MCP responses carry `Cache-Control: no-store` and `Content-Security-Policy: default-src 'none';
frame-ancestors 'none'`.
