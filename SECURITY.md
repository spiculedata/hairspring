# Security

Anthill is experimental software. It runs code you point it at and listens on a network port, so please read
this before deploying it.

## Reporting a vulnerability

Please report security issues **privately** using GitHub's "Report a vulnerability" (private security advisory)
on this repository, and do not open a public issue. Include the version or commit, what you did, and what you
observed. We will acknowledge reports as quickly as we can; this is a volunteer project and we make no
response-time guarantees.

## Threat model

The server may be reachable from a LAN or the internet behind bearer tokens. Callers hold one of three roles
(`read` < `run` < `admin`). Workers authenticate with a `run` token. We assume the **operator** is trusted: they
write `anthill.toml` (which defines the commands that process targets execute) and deploy the binaries.

We defend against: unauthenticated access, role escalation, command injection through the API, request
floods, malformed input, hostile web pages driving a local server, and untrusted text reaching people or AI
agents.

## What is implemented

| Area | Control |
|---|---|
| Authentication | Bearer tokens (16+ chars) stored only as SHA-256 digests; compared across all tokens in constant time |
| Authorization | Role checked first in every handler; worker endpoints require `run`; worker ids hidden from `read` tokens |
| No remote code execution | The API accepts **target names only**; commands come from the server-side `anthill.toml`. MCP `start_run` cannot supply a command |
| Network exposure | Refuses to bind a non-loopback address without tokens; default bind is `127.0.0.1` |
| Open (no-token) mode | Rejects unknown `Host` headers (DNS rebinding) and cross-site `Origin`s (CSRF); `/mcp` requires `application/json` |
| Abuse limits | Per-client rate limit (valid tokens and unauthenticated clients are bucketed separately, bounded table); 2 MiB body cap; cap on SSE streams, MCP batch size and waits; queue and worker admission caps; JSON recursion limits |
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
- **Process targets execute the commands in `anthill.toml` with the server's privileges.** Protect that file and
  run workers as an unprivileged user (the provided Docker image does).
- **Dependency vetting.** We use mainstream crates and npm packages and run `cargo audit` and `npm audit` in CI.
  We did not use a commercial software-composition-analysis service. Review `Cargo.lock` for your own policy.
- Not yet implemented: per-token run quotas, audit logging, token rotation without restart, S3 blob storage.
- The rate limiter is in-memory and per process; with several server replicas each enforces its own limit.

## Hardening checklist for a deployment

1. Generate tokens with `anthill token` and pass them via `ANTHILL_TOKENS` or `--tokens-file` (mode 600), never on a command line.
2. Terminate TLS in front of the server; keep PostgreSQL on a private network.
3. Run the server and workers as unprivileged users; give workers only the filesystem access their targets need.
4. Use `read` tokens for dashboards and agents that only analyse; `run` tokens only where needed.
5. Keep `anthill.toml` under version control with review.
