# Changelog

## Unreleased

## v0.1-RC2 (2026-11-05)

### Added

- Token labels. An entry in `--tokens-file` or `HAIRSPRING_TOKENS` may carry a label after the role, separated by one
  space (`token:run ci bot`). Entries without one are read as before. A label is 1 to 64 characters from ASCII
  letters, digits, space, `.`, `_` and `-`, must differ from every other label (ignoring case), and must not contain a
  configured token; otherwise the server does not start, and the error does not repeat the token. Only the operator
  can set a label. `GET /api/me` returns `label` to the holder of that token and to no one else, and the server's log
  lines for creating a run, starting a replay, registering a worker and changing a worker's trust now name the caller
  as `label (t:<key>)` or, without a label, by the key alone. A replay is logged once a replay slot is held. Actions
  taken through MCP over HTTP are not yet logged with the label. A label is not a secret and not proof of identity.
  The message for an unknown role no longer repeats the text in the role slot.
- `hairspring serve --trusted-proxy ADDR[,ADDR...]` (or `HAIRSPRING_TRUSTED_PROXY`) names reverse proxies, by
  address or network, whose `X-Forwarded-For` header gives the client address for the rate limit, for the check that
  refuses a valid token while its address's anonymous budget is used up, and, without tokens, for the per-caller
  event-stream cap. The header is read only on connections from a listed proxy, from the right, skipping listed
  proxies and stopping at the first address that is not one; entries further left are never decoded or parsed, so
  malformed text there, including bytes that are not ASCII or not valid UTF-8 on the same line as the proxy's
  entry, no longer causes a fallback. With a network of up to /8 (IPv4) or /32 (IPv6) listed, a client inside it
  can still choose its counted address through an appending proxy; SECURITY.md lists this and two further cases. A missing header, a malformed entry on the way to the client
  address, one that needs more than 1024 bytes or 16 entries counted from the right, or a header of only listed
  proxies falls back to the connecting address. `Forwarded` and `X-Real-IP` are not read, and the open-mode `Host`
  and `Origin` checks never use forwarding headers. Refused at startup: networks wider than /8 (IPv4) or /32 (IPv6),
  the unspecified address and ranges starting at it, and IPv4-mapped or IPv4-compatible IPv6 entries; a repeated
  entry is listed once, and an error message repeats at most 16 characters of the entry, control characters
  escaped; for an IPv4-mapped entry the message suggests the IPv4 form only when that form would be accepted,
  and the unspecified address and ranges starting at it are explained as addresses no proxy connects from. Without the flag nothing changes: forwarding headers are ignored and all callers
  behind a proxy share its address, so unauthenticated traffic through the proxy can cause valid tokens arriving
  through it to be refused. SECURITY.md describes how to configure nginx and Caddy to overwrite the header.
- `GET /api/workers` also reports, for each worker, a `state` (`busy`, `idle` or `not_seen_recently`), the parsed
  `capabilities`, the jobs it holds (`current_jobs`, at most three, and the count `leased_jobs`), and the response has a
  `queue` with the number of jobs waiting and the number held. The `read` role is enough; `id` is still left out for
  that role, and no secret, digest, job payload, error text or guest output is returned. Existing fields are unchanged.
  Both stores answer the same way (two new read-only queries on the job table; no schema change).
- `hairspring gc --older-than 30d` reports what retention would remove and removes nothing by default: runs by state,
  failures, stored check results, trials, events, jobs, trial trees, stored blobs and bytes, and bundle files in
  `--bundle-dir`. The report ends with `--apply --cutoff T --expect-runs N --expect-digest D`: the report's cutoff
  (milliseconds since 1970, from this computer's clock), the number of runs and a digest of their ids. With those, the
  apply looks at the runs that were old at that cutoff and removes them only if they are the runs the report showed;
  otherwise nothing is removed. A run that stops qualifying while it is being removed is kept and listed with its
  reason. Queued and running runs and runs with a queued or running job are never removed; failed runs are kept
  unless `--state failed` is given; a trial tree is removed whole or kept whole; blobs go only when nothing refers to
  them; a symbolic link is never followed, and a bundle file is removed only if the bundle directory and the file are
  still the ones that were read and hashed. `--json` prints one JSON document; exit 2 for an error, a refused
  `--apply`, an apply that removed fewer runs than the report showed, or a removal that stopped part-way. Run
  `--apply` when no worker is running jobs: a new failure whose bundle text is identical to one being removed can be
  left without its stored bundle. The report takes no lock that a worker or the server waits for (its counts are an
  estimate as of one snapshot, which the apply checks again); the apply works in short batches, each checking the
  blobs of its runs together, but each batch reads the whole trials table, so run it off-peak on a large store. Only
  one `gc` runs against a database at a time; a second one says another is running and exits 2. No schema
  change; SQLite and PostgreSQL behave the same. The database file does not shrink, and the UI still shows a removed
  run as "may have been removed" (no "expired" state yet). See "Retention" in `docs/architecture.md`.
- `GET /api/me` returns the caller's role (`read`, `run` or `admin`), what that role may do, and whether the server is
  in open mode. It needs a valid token like the other read endpoints (401 otherwise) and never includes the token or a
  digest of it.
- `GET /api/runs` and `GET /api/failures` page with `cursor` and `next_cursor`, as the property and trial lists do, and
  take filters: runs by `state`, `target` and `mode`; failures by `run`, `property` (a prefix) and `status`. `limit`
  defaults to 50 and is at most 500. A value that is not valid is now a 400 with a JSON body that names the parameter.
  A list can be read down to position 100000; a page that would continue past that has no `next_cursor`.
  Both stores give the same rows in the same order. See "Paging and filters on the runs and failures lists" in
  `docs/architecture.md`.
- `GET /api/search?q=...` finds ids and names: run ids, failure ids, property ids and target names by their start, and
  seeds and schedule fingerprints exactly. It returns hits in groups of at most 5, with `more` naming the groups that
  have others, and never returns guest output, bundles or failure messages. Runs and failures are newest first, targets
  in name order, properties by newest run and then by id within a run, seeds as the runs started with the seed and then
  the trials that ran it, and schedules by newest run and then by trial. The text is 1 to 64 characters from ASCII
  letters, digits and `_ - . : /`; anything else is a 400 that does not repeat it. It needs a valid token (any role) and
  counts against the rate limit. It looks in the 1000 newest runs (10 for property ids, which are not looked for when
  the text is a whole run or failure id, digits or a fingerprint), plus a direct lookup of a whole run or failure id of
  any age, so an empty answer means "none among the recent runs"; `scope` in the answer says how far, including how
  many runs' property catalogues were read. Both stores give the same hits. No schema change. See "Search" in
  `docs/architecture.md` for the measured worst-case cost.
- The web UI's top bar has a search box over `GET /api/search`: debounced, grouped hits (runs, failures, targets,
  properties, seeds, schedules) in a listbox you can drive from the keyboard (Down, Up, Enter, Escape), with an honest
  "No matches." that says how far the search looked. It fits a 320 px screen, shows property ids as escaped text, stops
  after a 429 until the server's retry time, and is not drawn at all when the server answers 404 to `/api/search`.
- A Hermit bundle records hashes of the target's command line and working directory (`target.command.sha256`,
  `target.cwd.sha256`; no paths or arguments are stored). Replay warns when either differs, still runs from the current
  configuration, and a miss then exits 3 (inconclusive). Older bundles replay as before and say the fields were not
  recorded.
- `hairspring test --sweep --confirm` runs the confirmation pipeline (the defaults of `hairspring confirm`) on every
  failure row the sweep stored, once the sweep ends, and prints each result. It works on the command line only: the
  server does not confirm sweeps by itself and workers do not run confirmations. A `sim` target is skipped with a
  note, since it cannot be confirmed, and the sweep carries on to the later targets with its own exit status.
- The Runs and Failures pages of the web UI have filter bars (runs by state, run type and target; failures by run id,
  property id prefix and state) kept in the address, and a **Load more** button that adds older pages. The newest page
  still refreshes by itself, a failed load keeps the rows already shown, and a list filtered by state shows a State
  column. The "Showing the newest 50 failures" note is gone: the server now says whether older failures exist. This
  needs the paging parameters of `GET /api/runs` and `GET /api/failures`.
- The run page's activity panel shows each event as a titled line with labelled facts instead of raw JSON, with the raw
  event behind a "Raw event" disclosure.
- The run page has four tabs under the run's facts: Properties, Seeds and trials, Failures (with a count) and Activity.
  The chosen tab is kept in the address as `?tab=...`, and the plain run address opens Properties. The tab row scrolls
  sideways on a phone. The run page now also shows the coverage chart, on the Properties tab, so it makes a new request,
  `GET /api/runs/{id}/coverage`, which is repeated while the run is live. The page's event stream stays open for the
  whole page, whichever tab is showing.
- `scripts/check-embedded-ui.sh` checks that a release binary serves the web UI built from the repository's `web/`
  source, and fails on an older UI. The release workflow runs it after each binary is built, `make release` runs it
  after the build, and `make check-ui` runs it on its own.
- `hairspring serve --organisation NAME` (or `HAIRSPRING_ORGANISATION`) shows an organisation name in the web UI's top
  bar. It is plain text of at most 80 characters, with no logo. `GET /api/health` carries it as `organisation`, only for
  a caller with a valid token (or any caller when the server has no tokens), and leaves the field out when no name is
  set. A health request that carries a token is counted as other endpoints count it: a valid token's health requests
  count against that token's own allowance, and a wrong token's against the client address's allowance. The field is
  left out (the answer is still 200) once the token's own allowance is used up, or while the address's allowance is,
  so health cannot be used to guess tokens, and one token polling health does not use up the address's allowance
  for other tokens. A request without a token is never limited.

- The web UI asks `GET /api/me` for the caller's role once per page load, and again after the token changes. When the
  role cannot start runs (`read`), the Runs page disables Start run and Start sweep and says so in one line, instead
  of waiting for a 403. If the server cannot answer, or takes longer than 5 seconds, the button stays available and
  the server still decides.
- The Workers page shows, for each worker, its state (Busy, Idle or Not seen recently), whether it is trusted, the
  capabilities it stated, the run it is holding and when it was last seen, with the number of jobs waiting and held
  above the table. The table sits in a scroll region so it fits a 320 pixel screen. A `read` caller sees no Id column
  and is told why (an id is a credential, shown from the `run` role). It only reads: trust is still set through
  `POST /api/workers/{id}/trust`. Against an older server that does not send the new state, capability, held-job and
  queue fields, State and Holding say "Not reported by this server" and there is no queue line; Trust and
  Capabilities still show from the `trusted` and `caps_json` fields it does send. A response with no list of workers
  shows an error instead of "Loading workers". Capability text is cut by character, never through the middle of one.

- A .NET guest SDK (`sdks/dotnet`, C#, base class library only, Linux and macOS) and a .NET language fixture. See
  `docs/dotnet-sdk.md`; `docs/support-matrix.md` gives its status.
- `docs/support-matrix.md` lists, per language, whether it passed the acceptance test under Hermit and on what
  configuration. The results apply only to that configuration.
- A Java service fixture (`svc.jar`) with an idempotency race. The Java lost-update fixture takes `--k=N`, the
  iterations per worker (1 to 5,000; the default stays 1,000). `K=<n>` and an `a-prime` JVM flag profile are available
  for the fixture's native checks, and a manual workflow, `java-native-k`, runs the fixture natively at a chosen K and
  profile.
- Web UI: one status badge and one status table render run states, property statuses and replay results; a word the
  table does not know shows as a neutral "Unknown", and run states such as "Queued" and "Failed" are now capitalised.
- Web UI: a Failures page listing stored failures, newest first, and a failure page with a read-only Confirmation ladder
  and a table of its checks. Neither starts a confirmation.
- Web UI: the Runs page can start a sweep as well as a search, with a Trials field for a sweep. The run page has a "What
  this run shows" and "What it does not show" panel, and a "Coverage over trials" chart (thinned to at most 2,000
  points, with "Show as a table").
- Web UI: an Overview page at `/` with the newest runs and failures that need attention and, when the server has no
  stored runs, a first-run guide. A Glossary page at `/glossary` with inline definitions on the failure page.
- Web UI: an app shell with side navigation and a System, Light or Dark theme switch. Polling pauses while the tab is
  hidden and refreshes when it is shown again, and a banner appears when the browser is offline or the server cannot be
  reached; the property, trial and strip loaders use the same loop. Failed requests back off up to 30 seconds.
- Web UI: the trial page has section navigation, taller thread lanes and an output viewer with line numbers and a find
  box. The failure and trial pages link to each other and have Copy link and Copy summary; a summary holds ids, names
  and figures, not program output.
- The service layer checks tree edge and command fields (digests, and a command that must be a non-empty JSON object of
  at most 64 KiB with no NUL and at most 32 levels) and refuses malformed worker input with a typed error naming the
  field. No server route calls these checks yet.
- The GitHub Action (`integrations/github-action`) can be used from another repository, pinned by commit SHA. It
  installs the release binary for the runner (Linux x64, macOS arm64) after checking it against the release's
  `SHA256SUMS`, makes one run and one report (`format`: `sarif`, `junit` or `text`), and takes `seeds` for a sweep
  range. Behaviour on GitHub's own runners has not been tested. `docker-compose.yml` now has the `.env.example` it
  refers to; its placeholders are shorter than the 16-character token minimum, so an unedited copy is refused at
  startup.
- `scripts/hermit-upstream-report.sh` prints a read-only report of what changed in upstream Hermit since the pinned
  commit (whether the replay patch still applies, new upstream commits, open upstream issues), with a `--self-test` that
  needs no network and runs in CI. A weekly workflow, `hermit-upstream.yml`, runs it and keeps the report in the job
  summary and as an artefact. Nothing is changed or opened by either.
- CI runs the PostgreSQL store, service and CLI tests against a PostgreSQL 16 service container, and a missing database
  fails the job instead of skipping the tests.
- Run inputs are recorded: for each run, a SHA-256 digest of each input it executed with (never the values, which can
  include the working directory and environment), whether the reporting worker was trusted, and the attempt of the job
  that ran. A job leased again records its own inputs, which replace the earlier attempt's. The process records them for
  a run of a built-in simulated target (the digest of the executable it runs, the target name and the backend), and a
  remote worker reports them to the new `POST /api/worker/jobs/{job}/inputs`, with its secret, for the job it holds; a
  report must name every input its scheme has and be for the run's own target and backend. Nothing serves a digest, and
  no existing response changes; a run stored before this version, or a Hermit or WASM run, has none and reads as "not
  recorded". Nothing compares runs by these inputs yet. See "Run inputs" in docs/architecture.md.

### Changed

- `hairspring --version` names the build: `hairspring 0.1.0 (f819c1a 2026-10-24)` is the crate version, the short
  commit and that commit's date, so two builds of the same crate version can be told apart (`v0.1-RC1` and later
  builds all printed `hairspring 0.1.0`). The commit comes from git at build time, or from `HAIRSPRING_BUILD_COMMIT`
  and `HAIRSPRING_BUILD_DATE`, which the release workflow sets and checks; without either it prints
  `hairspring 0.1.0 (unknown)`, and the build never fails for want of git. No wall-clock time is used. The line still
  starts `hairspring <version>`, which is what `scripts/install.sh` checks. The Dockerfile accepts the two values as
  build arguments, and the draft release notes give a checksum command that works with only one binary downloaded.
- Schema version 7 adds two indexes, on both stores, and nothing else: `failures_created` (the order of the failures
  list) and `run_properties_violation` (a partial index of the stored violations). Opening a version 6 database
  creates them in the transaction that sets the version; no row is rewritten. On PostgreSQL this blocks writes to
  `failures` and `run_properties` while the indexes build (0.05 to 0.18 s each on a store of 300000 failures; SQLite
  0.09 to 0.25 s), and it must be done by the role that owns the tables; a role that only reads and writes rows can
  still open a version 7 schema. An index of either name built beforehand is kept only if it has the same definition
  (and, on PostgreSQL, is valid): otherwise the open is refused and asks for it to be dropped, which is also what to do
  with the invalid index a failed `CREATE INDEX CONCURRENTLY` leaves. An older release refuses a version 7
  database. There is no offline downgrade from 7 to 6 (reset the stored version by hand); the downgrade to version 2
  drops the indexes. See "Failure list indexes" in
  docs/architecture.md.
- The `property` filter of `GET /api/failures` now looks at the newest 1000 runs only (or at the run named by `run`,
  however old), so its cost no longer grows with the stored history, only with the violations those runs hold (up to
  16384 per run in theory; see docs/architecture.md). A failure of an older run is not listed by it.
  With a `property` filter the answer carries `"scope": {"recent_runs": 1000, "cut_short": ...}`, where `cut_short` says
  that older runs were left out; without one the answer is unchanged. The web UI's hint for the filter says so, and
  the Failures page says when results were limited.
  Measured on a generated store of 100000 runs and 300000 failures (release builds, median of warm requests): the
  failures list without a filter took 100 to 109 ms on SQLite and 15 to 17 ms on PostgreSQL and now takes under 1 ms;
  `property=` took 0.8 to 1.9 s on SQLite for any prefix and 44 ms to 5.4 s on PostgreSQL and now takes 2 to 49 ms and
  10 to 225 ms; `status=confirmed` took 0.36 s and 55 ms and now takes 2 ms and 13 to 14 ms. Every other list,
  filter and cursor answers byte for byte as before on that store.
- The server keeps in memory the figures of a finished run that cost a pass over its stored rows: who reported its
  trials, its property statistics, its sweep counts, its trial count and its coverage series. An entry is served only
  while the run row read for the request is unchanged since the entry was worked out; trial rows cannot be added to a
  finished run, so they cannot change behind it, and a run removed by `hairspring gc` answers 404 as before. Checks of a
  run's failures, worker trust and the claim built from them are still read on every request. At most 32 runs (about 64
  MiB by estimate; a run of 16384 property ids takes about 7 MB with short ids and messages and about 23 MB when they
  are as long as allowed) and 64 coverage series are kept, least recently used first; restarting the server clears the
  cache. Concurrent first requests for one run work its figures out once, and if that fails they all get the same error
  rather than each trying again in turn. The coverage series of a run that is still open is kept once a scan saw no
  trial arrive while it ran; after that, polls with no new trial scan nothing, and a new trial row or a new attempt is
  scanned again. A series that is not kept is handed to the requests that waited for its scan only if the run's attempt
  was the same before and after the scan and the scan saw at least the trial rows counted before it; otherwise each of
  them scans for itself. Responses are unchanged byte for byte. Measured after the first request on a generated run at
  the service's limit of 10000 trials, with 16384 property ids: `GET /api/runs/{id}` took 55 ms on SQLite and 51 ms on
  PostgreSQL and now takes under 3 ms; the property table took 54 to 59 ms and 40 to 45 ms and now takes under 6 ms for
  any page or filter; a property's seed list still scans trials and dropped from 97 ms to 50 ms on SQLite and from 80 ms
  to 49 ms on PostgreSQL; repeated coverage polls of an open run of 10000 trials with no new trial took 55 ms and 53 ms
  and now take under 2 ms. On a generated store whose runs are larger than the service accepts (200000 trials) the run
  detail, the property table and the coverage polls went from between 79 and 276 ms to under 7 ms; with runs at the
  10000-trial cap the gains are smaller, as above. The first request for a finished run works all of its figures out at
  once and costs about what it did (59 ms against 58 ms on SQLite for the 10000-trial run). No schema change.
- `GET /api/failures` with a `status` filter reads the stored checks of the failures it scans in batches (one query per
  batch of up to 500 failures, with the ids as bound parameters) instead of one query per failure, and scans in
  batches of up to 500 failures instead of 100. Results, ordering, cursors and the scan cap of 1000 failures per
  request are unchanged. On a generated store of 300000 failures, `status=confirmed` took 1.75 s on SQLite and 0.39 s
  on PostgreSQL, and now takes 0.38 s and about 0.06 s. A page that a common status fills from the first failures
  reads more than before: `status=candidate` took about 30 ms more on SQLite. No schema change; the `Store` trait
  gains `list_checks_of_failures`.
- `GET /api/runs/{id}/events` reads only the run's own row, on connecting and at each poll, to check that the run
  exists and whether it is over, instead of building the full run detail. Events, their order, the end of the stream,
  role checks and the per-caller cap are unchanged, and so is the 404 for an unknown run. One case differs: when a
  run's full detail cannot be built (for example a stored check that cannot be decoded), connecting used to answer 500 and a poll ended the stream early; it now answers 200 and follows the run. On a
  generated run of 200000 trials the first byte took 0.4 s on SQLite (0.26 s to 0.76 s on PostgreSQL) and now takes under 1 ms (about 57 ms on PostgreSQL, where the
  first read of stored events is the rest), and eight simultaneous connects no longer take memory in proportion to the
  run's size.
- A SQLite store on a database file now reads through four read-only connections besides its one writer, so reads no
  longer wait for one another or for a write. Each read call sees one committed snapshot that includes every write that
  had returned before it began. A read of `/api/runs` while three clients looped the failures `status=confirmed` filter
  took 0.6 to 0.7 ms at the median instead of 264 to 371 ms (two runs of each, on 100,000 runs and 300,000
  failures; docs/architecture.md has the figures). In-memory stores keep a single connection; writes, retention and
  the schema are unchanged. The `hairspring` binary also turns off SQLite's process-wide memory statistics at start-up,
  whose mutex made heavy reads on several connections slower side by side than one after another (it prints one line
  to stderr if that setting could not be made). PostgreSQL is unaffected.

  Reads that overlap without a gap stop SQLite starting its WAL file again, so the store now limits it: when the WAL
  reaches 16 MiB the writer pauses the read connections, waits up to 250 ms for the reads in progress to finish,
  checkpoints with `TRUNCATE` (the file goes to zero bytes) and lends the connections again. A read that arrives
  during a pause waits at most 250 ms plus the time the checkpoint takes to copy the backlog, which was up to a few
  hundred milliseconds in the measurements (longest recorded pause 17.5 to 734 ms across runs, on a busy machine) and
  grows with a larger backlog; writes from the same process can also wait for the `PASSIVE` pass that comes first.
  If the reads in progress do not finish in time the checkpoint is skipped and tried again after 2 s. Any read still
  running 250 ms into each attempt causes a skip, so clients looping the heavy status filter (about 1.7 s) can keep
  the WAL growing, and each skip stalls reads and in-process writes for about 260 ms; during a long `gc --apply` the
  limit does not act until the writer is released, and the checkpoint after it copies the whole backlog while reads
  are paused. `journal_size_limit` is 4 MiB, and closing the store
  checkpoints. With 8 readers looping and an unthrottled writer of 2 KB events for 15 s, the WAL reached 3280 MiB
  without the limit and 16 MiB with it; the writer managed 8553 to 10292 writes/s instead of 10623.
- Server hardening. The server now closes a connection that has not sent a complete request head within 10 seconds
  (this is also the idle time allowed between requests on a kept-alive connection), answers `408` and closes the
  connection when a request body has not arrived within 30 seconds, and allows at most 512 open connections
  (`hairspring serve --max-connections N`); a connection beyond that is answered `503` with `Retry-After: 1` and
  closed (while 128 connections are being refused at once, a further one is closed without a reply). These
  deadlines do not apply while a response is being produced or sent, so event streams and long replays are
  unaffected. A request body is still read only after its token has been checked, so a request with a missing or
  wrong token is answered `401` without its body being read. The server speaks HTTP/1.1 only.
- At most 2 replays run on the server at once, REST `/api/replay` and MCP `replay_bundle` together
  (`hairspring serve --max-replays N`, 1 to 64). A replay over the limit is refused at once: REST answers `429` with
  `Retry-After: 5`, and the MCP tool returns a tool error with the code `replay_busy`.
- One caller (a token, or a client address when the server has no tokens) may hold at most 16 of the 64 event-stream
  slots; past that the stream request is answered `429`.
- The run event stream no longer shows worker ids to `read` tokens: the `worker` key is left out of event payloads
  for that role, as `GET /api/workers` already did.
- API and MCP responses carry `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'` (program output
  downloads keep `sandbox`), and the web UI's policy adds `base-uri 'none'` and `form-action 'self'`.
- A query string that does not parse (for example `?after=abc` on the event stream) is answered with the JSON error
  `{"error": "invalid query string"}` instead of a plain-text message.
- The per-client rate limit now also applies to valid tokens from an address that has used up its budget. Once
  requests without a valid token (no token, or a wrong one) from a client address (an IPv4 address or an IPv6 /64)
  go past `--rate-limit` in a minute, a request with a valid token from that address is answered `429` too. The
  window is fixed and restarts on the next request, so valid tokens from that address are refused while such traffic
  continues, and for up to a minute after it stops. This is the trade-off of limiting guessing per address, and it
  applies to every caller behind a shared address, such as a proxy; operators who need otherwise should put a
  limiter in front of the server. A valid
  token from any other address works as before, and its requests, including its `/api/health` requests, do not count
  against the address's allowance. `/api/health` without a token is still never limited; with a valid token from such
  an address it still answers 200, without the organisation name.

- Web UI wording and layout fixes. Pages no longer grow wider than the window when a table or a line of preformatted text
  is wide: the targets and workers tables and the bundle text scroll inside their own box, and a box that scrolls can be
  reached from the keyboard. While the connection banner is showing, a section whose request got no answer says
  "Could not load X until the connection returns" instead of repeating the browser's text; an error the server sent (a
  500, for example) is still shown as it came. The copy buttons on a page share one status message, and Copy bundle on
  the failure page now says when the copy did not work. A run whose coverage counts all stayed at zero shows one
  sentence ("Every count stayed at zero over N trials ... This records that none were counted, not that none exist")
  instead of three flat charts.
- The `not rated trials:` line in the text report of a search run or a sweep now says how many inconclusive trials
  showed a failure, counted once per trial. Those failures stay out of the hits and the rate. The count is stored
  with the run's property summary. A run with no stored property summary has an unknown count, and the line says
  nothing about it; summaries written before the count existed read it as 0. The count is a lower bound: it covers
  only timed-out trials whose events were read, and leaves out failures the run did not attribute to a stored per-id
  field, including ids dropped by the id cap. In a sweep the "N inconclusive" figure also includes unusable trials.

- A foreground command (`hairspring test`) now leases only the job of the run it started. It no longer runs, or marks
  as running, an older queued job of another run, such as one left behind by an earlier command that was killed with
  `kill -9`. Such jobs are left to workers (`hairspring worker`).
  The same applies to the MCP `start_run` tool when the server runs runs inline (`hairspring mcp`, which runs runs in
  its own process): it leases only the job of the run it starts, so it no longer runs other runs' queued jobs.
- `hairspring confirm <run-id> --json` now prints a JSON array with one entry per failure of the run. It used to print
  a single object. `hairspring confirm <failure-id> --json` and `hairspring confirm <bundle> --json` still print one
  object. Scripts that read the output for a run id need to read the first entry of the array, or loop over it.
- Web UI accessibility fixes found by an axe-core audit of every page in light and dark at 1280 and 390 pixels wide. The
  "What this run shows" and "What it does not show" headings on the run page are now level 2 (they skipped from level 1
  to level 3). A page, run, trial or failure that does not exist now has a level 1 heading ("No such page", "No such
  run", "No stored trial" for a seed with no stored trial, and so on) above the same message. On the seed explorer the
  table's scrolling box is named "Table of judged trials", so it no longer shares a name with the "Judged trials"
  section that holds the strip. The scrolling boxes for a raw event, the failure bundle and the output tail have the
  `group` role, so their names are announced. No result wording changed. A test over the main pages in the web UI's test
  suite runs axe-core (a new dev dependency of `web/`) on the markup rules; colour contrast and target size are not
  checked there.

- Schema version 6: a new run stores its mode, so sweeps are listed as sweeps by `GET /api/runs` and `GET
  /api/runs/{id}`. Opening a version 5 database sets the stored mode of past runs whose configuration says sweep to
  `sweep` (data only, no table or column changes). Older releases refuse a version 6 database, so back it up first; the
  previous release recorded sweeps as `search`. See "Stored run mode" in `docs/architecture.md`.
- Web UI: plainer result wording ("No failure seen", "Stopped early", "Failed again on replay"; a run that ran no trials
  never reads as a pass), colour tokens with an indigo accent and a contrast test, a wider figures column and a Runs
  table that fits a phone, and page titles, a skip link and absolute times with the relative time beneath.
- Java is marked supported on a Java-specific configuration in `docs/support-matrix.md`, with the configuration it
  passed on and the caveat that a replay matched only from the same paths as the recording.
- A sweep keeps a failure row for each property's first failing seed, not only for the lowest failing seed, and handles
  Ctrl-C.
- Every SQLite write transaction in the store starts with `BEGIN IMMEDIATE`, so a wait for another writer happens at the
  start of the transaction, under the 5 second busy timeout, instead of failing at once with `SQLITE_BUSY`.
- wasmtime is updated to 49.0.5, which fixes RUSTSEC-2026-0422, RUSTSEC-2026-0423 and RUSTSEC-2026-0424. The `wasm`
  backend uses only the core-module API, so the advisories were not reachable from it.
- The .NET SDK used for the fixture build record is pinned: `scripts/record-global.json` holds the exact version, and
  `global.json` at the repository root sets a floor of 10.0.100, so a newer SDK on a runner image is no longer used by
  the record.
- Schema version 8 upgrade (it adds the `run_inputs` table and the `runs_target_created` index, on both stores).
  Opening a version 7 database creates both in the transaction that sets the version; no row is rewritten. On SQLite
  every write to the database waits during the upgrade, up to the busy timeout, and then fails. On PostgreSQL
  `CREATE INDEX` takes a `SHARE` lock on `runs` until the migration commits, so every insert or update of a run waits
  (building the index beforehand with `CREATE INDEX CONCURRENTLY runs_target_created ON runs(target, created_at, seq)`
  avoids the long wait, but the `REFERENCES runs` in `CREATE TABLE run_inputs` still takes a brief lock on `runs`), and
  the migration must be run by the role that owns the tables. A role that only reads and writes rows needs
  `SELECT, INSERT, DELETE` on `run_inputs`, granted by the owner, before it can open a schema 8 database; without
  them the open is refused, before anything is changed, with the `GRANT` to run. A table or index of either name built
  beforehand is kept only if it has the same definition: otherwise the open is refused, everything is rolled back and the
  database stays at version 7. An older release refuses a version 8 database. There is no offline downgrade from 8 to
  7 (reset the stored version by hand); the downgrade to version 2 drops the table and the index. See "Run inputs" in
  docs/architecture.md.

### Fixed

- A process target whose program (`command[0]`) cannot be started is now a configuration error with a message naming
  the target and the program, `target NAME: program PATH not found or not executable` (exit 2), found before any trial
  runs. With the pinned Hermit a missing program was already an error, worded as a Hermit failure. Before any trial
  runs, the program is looked up on the host the way Hermit starts it by default: a name with
  a `/` relative to the target's `cwd`, a bare name on the guest's `PATH` (the target's own `PATH` if it sets one).
  When `hermit_args` contain `--mount`, `--bind`, `--tmp`, `--image`, `--env` (`-e`), `--base-env`, `--workdir`,
  `--config` or `--from`, the guest may see other files or another `PATH`, so a program missing on the host is only a
  warning and Hermit decides. After a trial, Hermit's own refusal to start the program (exit 127 with its "does not
  exist or is not accessible" or "Could not resolve program" message about the target's own program) is reported as the
  same configuration error, with Hermit's line added; a program that exits 127 itself is still judged by its exit
  status. The checks run where the backend runs, so a remote
  worker checks its own machine.
- `hairspring doctor --process-backend` no longer blames `perf_event_paranoid` when the kernel refuses a hardware
  counter on a machine with no hardware PMU registered (no `cpu` or `cpu_core` under
  `/sys/bus/event_source/devices`). It now says that the machine cannot provide the counters Hermit needs and that
  lowering the setting will not help. When that directory cannot be read at all (a chroot or sandbox without `/sys`),
  it says the cause cannot be determined. The advice about the setting stays for machines that have a PMU. Exit codes
  are unchanged.
- `hairspring worker --token-file` refuses a file holding more than one token or a `token:role` line, saying that the
  worker token file must contain one bare token; before, every request failed with an HTTP header error. The worker
  prints `serving` only after the server has accepted its registration.
- `hairspring doctor` no longer prints `x 0 steps` for wasm and process targets, which have no step count:
  `deterministic over 8 seeds`.
- `scripts/install.sh`: the help and the tag error now give `v0.1-RC1` as the example tag (a release that exists).
  `--dry-run --version TAG` checks that the release's `SHA256SUMS` exists (a HEAD request; status code only) and exits
  1 with `release TAG not found` when it does not; when there is no answer it warns that it could not check and still
  exits 0. A source build (and its dry run) checks for cargo and a C compiler and linker (`cc`) before building, and
  says what to install (`sudo apt-get install -y build-essential` on Debian and Ubuntu, `xcode-select --install` on
  macOS); `--with-ui` also checks for `node` and `npm`. A failing `cargo build` now exits 1 with `the build failed`
  rather than cargo's own status, so the installer keeps to its documented exit codes 0, 1 and 2.
- The `hairspring init` template's process example now shows `wall_secs = 600` in its budget, with a note that the
  default of 60 seconds allows few Hermit trials.
- Text a worker sends is now cleaned or refused when it arrives. A failure reason (`/api/worker/jobs/{job}/fail`) and
  a progress message are cut to one line with no control or bidirectional characters, as a result's error already was,
  so the run's `error` and the progress events carry clean text. A worker name with a control or invisible formatting
  character is a 400 that does not repeat the name. The line and paragraph separators (U+2028, U+2029) are now treated
  like other line breaks everywhere those characters are removed or refused, including the organisation name (the
  server and the web UI use the same list).
- Error messages from the REST API and the MCP tools that repeat part of a request (an unknown target, a bundle's
  seed or path, a run mode, a field of a request body that could not be read) are now one line with no control or
  bidirectional characters and at most 500 characters. A message that was cut ends in `...`. Messages for ordinary
  input read as before. The environment-mismatch message (409, `environment_mismatch`) follows the same rule and now
  also ends in `...` when it was cut, as does a property id longer than 500 characters in a `GET /api/search` answer.

- The Java SDK reopens a regular-file stderr only when `/proc/self/fdinfo/2` says it was opened for writing; with no
  fdinfo (macOS) or a read-only descriptor the file is never written.
- A trial that runs past its time limit no longer leaves temporary files in the host temp directory when `--base-env` is
  `minimal` or `empty`: each Hermit trial gets a private directory that the engine removes itself. With the default base
  environment, Hermit's private guest `/tmp` and `/etc/group` copy can still be left behind.
- A strict verify accepts a guest that exits 0 (it passed `--verify-allow=failure`, which refused one) and reports why a
  verify did not run; it also requires that the verify run failed the way the recording did. Its Fail wording in the
  checks table and the line for several failed properties were corrected, and the staging file
  `.hermit-verify-summary-*` is removed from the working directory after a verify is killed, except one Hermit put in
  `ignored/` of an enclosing git work tree.
- The service fixture's build record is shown in the CI step summary even when the record script fails.
- C SDK: the events file (`HAIRSPRING_EVENTS_FILE`) is opened non-blocking and must be a regular file; a FIFO or a
  device is refused, the event is counted in `end.dropped` and the report falls back to stderr. Before, a FIFO with no
  reader hung.
- C SDK: with g++ in C++ on ELF x86-64 and aarch64, call sites inside C++ templates are catalogued; with g++ 13.3 they
  were missing from the `catalog` and `assert` lines, and a file with both an inline-function site and an
  ordinary-function site was rejected.
- Trial-directory clean-up uses `O_DIRECTORY` and `O_NOFOLLOW` values chosen for each listed Linux architecture (x86,
  x86-64, riscv, loongarch, arm, aarch64, powerpc), stops the build on any other Linux architecture instead of using
  numbers that mean other flags, and walks directories through `/proc/self/fd`.
- The web UI rounds rates as the CLI report does (upper bounds up, lower bounds down, `<0.01%` below 0.01%), so 0 hits
  in 300 rated trials reads 1.00% in both. Rate text prints counts without thousands separators, as the server's does.
- The service-layer check refuses a `command_json` in which any object repeats a key, with an error distinct from
  malformed JSON. No server route calls it yet.

### Documentation

- `docs/supply-chain.md` and SECURITY.md note a known advisory (RUSTSEC-2026-0344, `ruzstd`) in the pinned Hermit
  build, when it can be reached, and that the fix will ship with the next pinned Hermit update.
- The Developer Certificate of Origin (`DCO`) and a sign-off statement in `CONTRIBUTING.md`: contributions are accepted
  under Apache-2.0 with a `Signed-off-by` line on every commit. There is no automated check yet.
- README and `docs/quickstart.md`: a web UI tour with screenshots (`docs/images/ui/`) and
  `scripts/capture-ui-screenshots.mjs`, which regenerates them in a scratch directory.
- `docs/quickstart.md` runs the install scripts with `bash` (they are not executable files), uses the `wrk_` and `run_`
  id prefixes the server issues, and the README names all 13 MCP tools.
- `docs/architecture.md` documents the per-token worker registration limit (`--register-limit`, 100 an hour by default,
  `0` disables). SECURITY.md says why anonymous IPv6 limiting stays at a /64.
- New guides: `docs/ci.md` (Hairspring in CI: a workflow with the GitHub Action, the same job without it, a regression
  sweep and its exit codes, process targets on a self-hosted Linux x86-64 runner, other CI systems, replaying a CI
  failure) and `docs/api.md` (an HTTP API reference with every route, the role it needs, paging and filters, the event
  stream and the error and limit codes). The REST sections that were in `docs/quickstart.md` moved to `docs/api.md`,
  and the quickstart now shows how to check a single downloaded binary against `SHA256SUMS`.
- `docs/deploy.md` (operating a server: tokens, remote workers, PostgreSQL, systemd units, logs, backups, retention,
  reverse proxy, security checklist) and `docs/upgrading.md` (telling builds apart, the schema-version error, upgrading
  safely, uninstalling). The bug report template asks for the `hairspring --version` line and the output of `hairspring
  doctor --process-backend`, and the issue chooser links to the wiki.
- Four guides under `docs/guides/` (is my machine suitable for the `process` backend, building a Linux executable to
  test, testing a service with a driver command, testing logic with the `wasm` backend), and corrected install snippets
  for the Go, Rust, .NET and Python SDKs.
- The README links to the CI guide, the HTTP API reference, the operating and upgrading pages and a getting-help
  section.
