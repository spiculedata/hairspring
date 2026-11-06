# Security

Hairspring is experimental software. It runs code you point it at and listens on a network port, so please read
this before deploying it.

If you run a server, [docs/deploy.md](docs/deploy.md) is the step-by-step guide (tokens, workers, PostgreSQL, systemd,
backups, upgrades); this page is the threat model, the limits and the reverse-proxy and hardening details.

## Reporting a vulnerability

Please report security issues **privately** using GitHub's "Report a vulnerability" (private security advisory)
on this repository, and do not open a public issue. Include the version or commit, what you did, and what you
observed. We will acknowledge reports as quickly as we can; this is a volunteer project and we make no
response-time guarantees.

## Threat model

The server may be reachable from a LAN or the internet behind bearer tokens. Callers hold one of three roles
(`read` < `run` < `admin`). Workers authenticate with a `run` token. We assume the **operator** is trusted: they
write `hairspring.toml` (which defines the commands that process targets execute) and deploy the binaries.

We defend against: unauthenticated access, role escalation, command injection through the API, request
floods, malformed input, hostile web pages driving a local server, and untrusted text reaching people or AI
agents.

## What is implemented

| Area | Control |
|---|---|
| Authentication | Bearer tokens (16+ chars) stored only as SHA-256 digests; compared across all tokens in constant time |
| Authorization | Role checked first in every handler; worker endpoints and replay (which executes target code) require `run`, as does `POST /api/failures/{id}/confirm`, which is off unless `--max-confirmations` is set; worker ids hidden from `read` tokens (left out of the worker list and of event-stream payloads) |
| No remote code execution | The API accepts **target names only**; commands come from the server-side `hairspring.toml`. MCP `start_run` cannot supply a command |
| Network exposure | Refuses to bind a non-loopback address without tokens; default bind is `127.0.0.1` |
| Open (no-token) mode | Rejects unknown `Host` headers (DNS rebinding) and cross-site `Origin`s (CSRF); `/mcp` requires `application/json` |
| Abuse limits | Per-client rate limit (valid tokens and unauthenticated clients are bucketed separately, bounded table; a valid token is also refused while its client address has used up its unauthenticated budget; the client address is the connecting one, or, only for connections from proxies named with `--trusted-proxy`, the right-most untrusted `X-Forwarded-For` entry); 2 MiB body cap; 10 s to send a request head (also the idle time allowed between requests on a kept-alive connection) and 30 s to send a body; bodies are read only after the token check; at most 512 open connections (`--max-connections`), 503 beyond; at most 2 replays running at once, REST and MCP together (`--max-replays`, at most 64), refused with 429 and `Retry-After` beyond, never queued; when `--max-confirmations N` is set (default 0 = off, at most 4), at most N confirmations at once and one per failure, 429 or 409 beyond, never queued; event streams capped at 64 in total and 16 per caller (a token, or a client address when there are no tokens); cap on MCP batch size and waits; queue and worker admission caps; JSON recursion limits |
| Worker data | Results from workers are size-bounded, control characters stripped, failure target checked against the job, written atomically; leases are owner-checked and heartbeated |
| Untrusted text | Step labels, states, messages and errors are treated as data. The UI renders them as text only; MCP output and instructions tell agents never to follow instructions found in them |
| Storage | All SQL is parameterised; blobs addressed by validated hash; database URLs and tokens are never printed or logged; `--tokens-file` permissions are checked |
| Responses | `nosniff`, `no-referrer`, `no-store` and `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'` on the API (program output downloads keep their own `sandbox` policy); CSP with `base-uri 'none'`, `form-action 'self'` and frame denial on the UI; errors, including a query string that does not parse, are JSON; internal errors are logged server-side and reported generically |
| Process targets | Guests start from a minimal pinned environment (the host environment, including secrets, is not inherited); output and runtime are bounded; hung process groups are killed |

## Known limitations (read these)

- **Plain HTTP/TCP.** There is no built-in TLS. Run behind a TLS-terminating proxy or on a private network.
  The same applies to the PostgreSQL connection (no TLS) and to the worker's token.
- **Token labels are names, not authentication.** An operator may add a label to a token in the token configuration
  (`token:role label`, see [docs/deploy.md](docs/deploy.md#token-labels-optional)). It is set only there and never
  from a request. It is not a secret and does not prove who is calling: people who share a token share its label, and
  whoever holds a token is shown that token's label by `GET /api/me`. It is shown to no other caller. It is written to
  the server's log lines for creating a run, starting a replay, registering a worker and changing a worker's trust,
  next to the token's key, never the token. Do not put a secret or personal data in a label. A label is restricted to
  ASCII letters, digits, space, `.`, `_` and `-`, at most 64 characters, must differ from every other label, and is
  refused if it contains a configured token. Those log lines cover REST calls; actions taken through MCP over HTTP
  are not yet logged with the label. The `t:<key>` in a log line is a prefix of the token's SHA-256 digest, so use
  tokens made by `hairspring token`: a weak, operator-chosen 16-character token could be guessed offline by someone
  who can read the logs.
- **Run tokens are a trust domain.** A worker id is not bound to the token that registered it, and a malicious
  `run` token can lease jobs and report arbitrary (but bounded) results. Give `run` tokens only to workers you trust.
- **Hermit is third-party code** (a small, fast-moving fork). It is unaudited by us and executes the program
  under test with ptrace and namespaces. Use it on machines where that risk is acceptable.
- **The pinned Hermit build has a known advisory.** It includes `ruzstd` versions affected by RUSTSEC-2026-0344 (a
  decompression bomb in the zstd decoder; denial of service only, CVSS 7.5). Hermit decodes zstd only when it reads
  compressed debug sections, which happens with its `--happens-before` and stack-trace options; Hairspring does not
  pass these by default, though `hermit_args` can. The effect is that run failing for lack of memory. A fix is carried
  on the fork and will ship with the next pinned update. See [supply-chain.md](docs/supply-chain.md).
- **Process targets execute the commands in `hairspring.toml` with the server's privileges.** Protect that file and
  run workers as an unprivileged user (the provided Docker image does).
- **Dependency vetting.** We use mainstream crates and npm packages and run `cargo audit` and `npm audit` in CI.
  We did not use a commercial software-composition-analysis service. Review `Cargo.lock` for your own policy.
- Not yet implemented: per-token run quotas, audit logging, token rotation without restart, S3 blob storage.
- **The connection cap is server-wide, not per address.** A client that opens many connections can fill the cap
  (512 by default), and other clients are answered `503` until those connections end. A connection that sends
  nothing, or sends a request head slowly, is closed after 10 seconds, and one that sends its body slowly after a
  further 30. A client that keeps sending requests on its kept-alive connections can hold them indefinitely; only
  per-client limits in front of the server prevent that. A proxy in front of the server makes all its
  clients share its connections, so give the proxy its own per-client connection limits. Up to 128 refused
  connections are held for at most a second each while they are told the server is busy, so keep the
  file-descriptor limit (`LimitNOFILE` under systemd) well above `--max-connections` plus 128.
- **A refused replay is not retried for you.** Replays beyond `--max-replays` get `429` (REST) or a `replay_busy`
  tool error (MCP) at once. A replay that is running keeps its slot until the target finishes, even if the client
  disconnects, so a `run` token can keep the replay slots busy for the length of the target's own timeouts; other
  endpoints are unaffected.
- **Confirming from the browser (`--max-confirmations`, off by default) lets any `run` token move a claim either way,
  and spends the server's machine time.** A confirmation runs the failure's `process` target on the server, up to
  3 reruns and a replay within 120 seconds, and the server records the checks as its own trusted runner, the same as
  `hairspring confirm` run there. Those checks decide where a failure stands and whether a claim counts it, so with
  the setting on, every holder of a `run` token can cause a confirmed failure to become not reproducible, or the
  other way round, on any failure the server's configuration can run. The caller chooses no command, budget or
  option: the input is a failure id. Every way of confirming a stored failure refuses one whose stored bundle is for a different target than the
  failure, so a worker cannot get checks recorded against a passing target by reporting another target's bundle. At most N confirmations run at once (N from 1 to 4), a failure has one at a
  time, and a request beyond either is refused (`429` with `Retry-After`, or `409`) and never queued, but a `run`
  token can keep all N slots busy for about two minutes at a time, repeatedly, until per-token quotas exist (listed under
  "Not yet implemented" above). Enable it only on a deployment where every `run` token holder is trusted with that, and size
  N for the machine: each running confirmation holds a blocking thread and a Hermit process group. A confirmation whose every check is inconclusive (for example
  Hermit is missing on the server) still counts as finished and replaces the finding from the newest finished
  confirmation, so it can move a confirmed failure back to candidate. It does not run on
  remote workers, and the server logs who started each one as `label (t:<key>)`, never the token. Leave it at 0 on a
  server that anyone you do not trust can reach with a `run` token.
- The rate limiter is in-memory and per process; with several server replicas each enforces its own limit.
- **Anonymous rate-limit buckets are keyed by the connecting address.** IPv4 callers get one bucket per address and
  IPv6 callers one per /64. A holder of a larger IPv6 allocation (a /56 or /48) therefore gets a separate bucket for
  every /64 it controls, and can also push older anonymous windows out of the bounded table by sending from many
  /64s. This matters only where anonymous callers reach real endpoints (open mode, with no tokens configured).
  Valid tokens keep their own buckets and are never evicted by anonymous traffic. The bucket is not widened to a /56
  or /48 because delegations differ: a hosting provider commonly gives each customer a /64 out of a shared /56 or
  /48, so a wider bucket would make unrelated customers share one allowance and throttle each other. A /64 is the
  smallest unit that one subscriber or host controls, and the cost of keeping it is the weakness described above. If
  you need a coarser limit for anonymous IPv6 callers, enforce it at a proxy that knows your clients' allocations, or
  configure tokens so that open mode is not in use.
- **A valid token can be refused because of other traffic from its address.** Requests to `/api/` (except
  `/api/health`) and `/mcp` without a valid token (no token, or a wrong one) count against the client address's
  anonymous bucket. Once that bucket is over the limit, a request with a valid token from the same address (the same
  IPv4 address or IPv6 /64) is also answered `429` with `Retry-After: 60`, without spending the token's own allowance,
  while that bucket stays over the limit. Without this, the right token would be told apart from wrong guesses by
  its `200`, and the limit would not slow guessing. The window is fixed and restarts with the first request after
  it ends, so valid tokens from that address are refused while such traffic continues, and for up to a minute after
  it stops. This is the trade-off of limiting guessing per address: a legitimate caller sharing an address (behind
  the same NAT or proxy, or in the same /64) is affected by what else arrives from it. Operators who need a
  different trade-off should put a rate limiter in front of the server. A valid
  token from a quiet address is unaffected, and its requests never spend the address's anonymous budget. The address
  is the connecting one, and forwarding headers are ignored, unless the connection comes from a proxy named with
  `--trusted-proxy` (see "Running behind a reverse proxy" below). `/api/health` follows the same rule for a request that
  carries an `Authorization` header: a valid token's health requests count against that token's own bucket, anything
  else counts against the address's anonymous bucket, and the organisation name is left out (the answer is still
  `200`) once the token's own bucket is used up or while the address's bucket is over the limit. Health without the
  header is never counted. Two limits remain: the anonymous window can be pushed out of
  the bounded table by traffic from very many addresses (see above), which clears the refusal early, and the check
  covers one address only, so guessing spread over many addresses gets one allowance per address.
- **Behind a reverse proxy, without `--trusted-proxy`, all callers share the proxy's address.** By default
  forwarding headers such as `X-Forwarded-For` are not trusted (a client could set them to mint fresh buckets at
  will), so every caller arriving through the proxy is counted under the proxy's address. Authenticated callers are
  keyed by token, but once unauthenticated callers behind the proxy use up that shared anonymous bucket, every valid
  token arriving through the proxy, workers included, is refused while that traffic continues, and for up to a minute
  after it stops (see the previous point). Name the proxy with `--trusted-proxy` to avoid this, as described below.

## Running behind a reverse proxy

There is no built-in TLS, so a deployment reachable beyond one machine normally sits behind a TLS-terminating
reverse proxy. Tell the server which proxies to believe with `hairspring serve --trusted-proxy ADDR[,ADDR...]` (or
the `HAIRSPRING_TRUSTED_PROXY` environment variable; the flag wins, and an empty flag trusts none). Each entry is an
address (`10.0.0.5`, `2001:db8::5`) or a network (`10.0.0.0/24`, `2001:db8::/64`). At startup the server prints the
list it trusts.

- Only a request whose connection comes from a listed proxy has its `X-Forwarded-For` header read. From anywhere
  else the header is ignored and the connecting address is used, exactly as without the flag.
- The header is read from the right: entries that are themselves listed proxies are skipped, and the first entry
  that is not is the client address. Reading stops there; anything further left is never decoded or parsed, so
  text there (including bytes that are not ASCII or not valid UTF-8, even on the same line as the address the proxy
  appended) cannot cause a fallback. Whether it can change the result is covered under "What remains". Several `X-Forwarded-For` headers are read as one list, in order.
- The connecting address is used instead when the header is missing, when an entry read on the way to the client
  address is not a plain IP address (a port, brackets, `unknown`, a zone, an empty entry), when finding the client
  address would need more than 16 entries or 1024 bytes counted from the right, or when every entry is a listed
  proxy. The total size of the header is not limited by this check (the HTTP server limits it), and a long header
  does not cause a fallback by itself.
- The standard `Forwarded` header and `X-Real-IP` are not read.
- The client address keys the anonymous rate-limit bucket (IPv4 address or IPv6 /64, as before), the check that
  refuses a valid token while its address's anonymous bucket is over the limit, the same check at `/api/health`, and,
  when the server has no tokens, the per-caller event-stream cap. The open-mode `Host` and `Origin` checks never
  look at forwarding headers, and open mode still listens on loopback only.
- Refused at startup, with a message that names the setting: any network wider than /8 (IPv4) or /32 (IPv6), which
  includes `0.0.0.0/0` and `::/0`; the unspecified address (`0.0.0.0`, `::`) and any range starting at it
  (`0.0.0.0/8`, `::/96`); a network with bits set past its prefix (`10.0.0.5/24`); an IPv4-mapped (`::ffff:10.0.0.5`)
  or IPv4-compatible (`::10.0.0.5`) IPv6 entry (write it as IPv4; the message suggests the IPv4 form only when that
  form would be accepted, with the prefix reduced by 96 for a network); and anything that is not an address or a network
  in the forms above. A repeated entry is listed once. An error message repeats at most the first 16 characters of
  the entry, with control characters escaped.

The proxy must **overwrite** any `X-Forwarded-For` the client sent with the address it saw, not append to it. For
nginx:

```nginx
location / {
    proxy_pass http://127.0.0.1:7878;
    # Overwrite: the client's own X-Forwarded-For is discarded.
    proxy_set_header X-Forwarded-For $remote_addr;
    proxy_set_header Host $host;
    # Event streams: do not buffer.
    proxy_buffering off;
}
```

Do not use `$proxy_add_x_forwarded_for` on the proxy that faces clients; it appends. For Caddy:

```caddy
hairspring.example.com {
    reverse_proxy 127.0.0.1:7878 {
        # Overwrite with the address of the connecting client.
        header_up X-Forwarded-For {remote_host}
    }
}
```

Caddy 2.5 and later already replace `X-Forwarded-For` from clients it does not trust unless its own `trusted_proxies`
option is set, so the `header_up` line is harmless but may be redundant; check your Caddy version. Nginx closes a
connection to the server after `proxy_read_timeout` (60 seconds by default) without data, which can end a quiet event
stream; raise it for the `/api/` event streams (for example `proxy_read_timeout 1h;` in the location that serves
them).

Start the server with `--trusted-proxy 127.0.0.1` (or the proxy's address on your private network). When there
is more than one proxy (a load balancer in front of nginx, say), the one facing clients overwrites, the inner ones
append, and every one of them is listed.

What remains:

- **A proxy that appends instead of overwriting** still puts the address it saw right-most. When that address is not itself
  a listed proxy it is the client address, and whatever the client wrote to the left of it is never decoded or parsed
  and cannot change the result. Overwriting is still the recommended configuration: it keeps the header short and
  does not depend on every proxy in the chain appending correctly. A proxy that passes the header through without
  adding anything is the case described below.
- **A client inside a listed network can choose its counted address through an appending proxy.** Networks up to
  /8 (IPv4) or /32 (IPv6) can be listed. If the address the proxy saw is inside such a network, the proxy appends it,
  it is skipped as a listed proxy, and reading continues into text the client wrote, so the client names the address it
  is counted under. List the narrowest network that covers your proxies, and do not list a network that also holds
  clients.
- **Two further cases remain, and are documented here rather than detected.** A listed proxy that forwards a request
  without adding the address it saw leaves the client's own header as the right-most entry. A host inside a listed
  network that connects to the server directly is a trusted peer, so its header is read and believed. Both let that
  host choose its address, as in the next item.
- **Listing an address a client can send from**, or a proxy that passes a client's header through unchanged, lets
  that client choose any address. It can then spread guesses over many buckets, and fill the limiter's table. The
  table holds at most 10,000 windows; when it is full of live windows the oldest tenth of the anonymous ones is
  dropped, which ends those addresses' refusals early. Token buckets are never dropped. List only proxies you
  control, and keep the server's port unreachable except through them (a direct connection is counted under its own
  address, so this matters for load, not for spoofing).
- One client address behind a NAT, or one IPv6 /64, is still one bucket.

## Hardening checklist for a deployment

1. Generate tokens with `hairspring token` and pass them via `HAIRSPRING_TOKENS` or `--tokens-file` (mode 600), never on a command line.
2. Terminate TLS in front of the server; keep PostgreSQL on a private network.
3. Run the server and workers as unprivileged users; give workers only the filesystem access their targets need.
4. Use `read` tokens for dashboards and agents that only analyse; `run` tokens only where needed.
5. Keep `hairspring.toml` under version control with review.
6. Set the file-descriptor limit well above `--max-connections` (512 by default), and give a proxy in front of the
   server its own per-client connection and rate limits.
7. Behind a reverse proxy, name it with `--trusted-proxy` and configure it to overwrite `X-Forwarded-For` (see
   "Running behind a reverse proxy").
