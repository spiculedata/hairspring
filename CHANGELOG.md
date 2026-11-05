# Changelog

## Unreleased

### Added

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

### Changed

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

### Fixed

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

### Documentation

- `docs/supply-chain.md` and SECURITY.md note a known advisory (RUSTSEC-2026-0344, `ruzstd`) in the pinned Hermit
  build, when it can be reached, and that the fix will ship with the next pinned Hermit update.
