# Security

Hairspring is experimental software. It runs code you point it at and listens on a network port, so please read
this before deploying it.

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
| Authorization | Role checked first in every handler; worker endpoints and replay (which executes target code) require `run`; worker ids hidden from `read` tokens |
| No remote code execution | The API accepts **target names only**; commands come from the server-side `hairspring.toml`. MCP `start_run` cannot supply a command |
| Network exposure | Refuses to bind a non-loopback address without tokens; default bind is `127.0.0.1` |
| Open (no-token) mode | Rejects unknown `Host` headers (DNS rebinding) and cross-site `Origin`s (CSRF); `/mcp` requires `application/json` |
| Abuse limits | Per-client rate limit (valid tokens and unauthenticated clients are bucketed separately, bounded table; a valid token is also refused while its client address has used up its unauthenticated budget); 2 MiB body cap; cap on SSE streams, MCP batch size and waits; queue and worker admission caps; JSON recursion limits |
| Worker data | Results from workers are size-bounded, control characters stripped, failure target checked against the job, written atomically; leases are owner-checked and heartbeated |
| Untrusted text | Step labels, states, messages and errors are treated as data. The UI renders them as text only; MCP output and instructions tell agents never to follow instructions found in them |
| Storage | All SQL is parameterised; blobs addressed by validated hash; database URLs and tokens are never printed or logged; `--tokens-file` permissions are checked |
| Responses | `nosniff`, `no-referrer`, `no-store` on the API; CSP and frame denial on the UI; internal errors are logged server-side and reported generically |
| Process targets | Guests start from a minimal pinned environment (the host environment, including secrets, is not inherited); output and runtime are bounded; hung process groups are killed |

## Known limitations (read these)

- **Plain HTTP/TCP.** There is no built-in TLS. Run behind a TLS-terminating proxy or on a private network.
  The same applies to the PostgreSQL connection (no TLS) and to the worker's token.
- **Run tokens are a trust domain.** A worker id is not bound to the token that registered it, and a malicious
  `run` token can lease jobs and report arbitrary (but bounded) results. Give `run` tokens only to workers you trust.
- **Hermit is third-party code** (a small, fast-moving fork). It is unaudited by us and executes the program
  under test with ptrace and namespaces. Use it on machines where that risk is acceptable.
- **Process targets execute the commands in `hairspring.toml` with the server's privileges.** Protect that file and
  run workers as an unprivileged user (the provided Docker image does).
- **Dependency vetting.** We use mainstream crates and npm packages and run `cargo audit` and `npm audit` in CI.
  We did not use a commercial software-composition-analysis service. Review `Cargo.lock` for your own policy.
- Not yet implemented: per-token run quotas, audit logging, token rotation without restart, S3 blob storage.
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
  until the bucket's one-minute window ends. Without this, the right token would be told apart from wrong guesses by
  its `200`, and the limit would not slow guessing. The cost is that a legitimate caller sharing an address with
  someone guessing (behind the same NAT or proxy, or in the same /64) can be refused for up to a minute. A valid
  token from a quiet address is unaffected, and its requests never spend the address's anonymous budget. The address
  is the connecting one; forwarding headers are ignored. `/api/health` follows the same rule for a request that
  carries an `Authorization` header: a valid token's health requests count against that token's own bucket, anything
  else counts against the address's anonymous bucket, and the organisation name is left out (the answer is still
  `200`) once the token's own bucket is used up or while the address's bucket is over the limit. Health without the
  header is never counted. Two limits remain: the anonymous window can be pushed out of
  the bounded table by traffic from very many addresses (see above), which clears the refusal early, and the check
  covers one address only, so guessing spread over many addresses gets one allowance per address.
- **Behind a reverse proxy, anonymous callers share one bucket.** Forwarding headers such as `X-Forwarded-For` are
  deliberately not trusted (a client could set them to mint fresh buckets at will), so every anonymous caller
  arriving through the proxy is counted under the proxy's address. Authenticated callers are keyed by token, but
  once the anonymous callers behind the proxy use up that shared bucket, every valid token arriving through the proxy
  is refused for the rest of the minute too (see the previous point). If you need per-client limits for anonymous
  traffic, enforce them at the proxy.

## Hardening checklist for a deployment

1. Generate tokens with `hairspring token` and pass them via `HAIRSPRING_TOKENS` or `--tokens-file` (mode 600), never on a command line.
2. Terminate TLS in front of the server; keep PostgreSQL on a private network.
3. Run the server and workers as unprivileged users; give workers only the filesystem access their targets need.
4. Use `read` tokens for dashboards and agents that only analyse; `run` tokens only where needed.
5. Keep `hairspring.toml` under version control with review.
