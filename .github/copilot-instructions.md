# GitHub Copilot: Hairspring

Deterministic simulation testing: a Rust workspace (`crates/`) with a React and TypeScript web UI
(`web/`, Vite and Vitest) embedded into the server, and SDKs in `sdks/` for C, Go, Java, .NET, Python
and Rust. Experimental software.

Full map and rules: `AGENTS.md` at the repo root. Setup and sign-off: `CONTRIBUTING.md`. Threat
model: `SECURITY.md`.

## Layout, so you suggest changes in the right place

- `crates/hairspring-core`: the `Sim` trait, explorer, shrinker, bundle format. Its only dependency is
  `sha2`; do not add another.
- `crates/hairspring-engine`, `-store`, `-service`: engine and backends, persistence (SQLite and
  PostgreSQL), and the one place that changes run and job state. Front ends (`-cli`, `-server`, `-mcp`,
  `-worker`) depend on the service, never the other way round.
- `crates/hairspring-hermit`, `-wasm`: the other backends. `-props`, `-check`: pure parsers and checkers.
- `web/`: the UI. Untrusted text (step labels, states, messages) is rendered as text only.

## Conventions to match

- Tests first; a bug fix starts with a test that reproduces it.
- A change to what is stored updates both `sqlite.rs` and `pg.rs` in `hairspring-store` and the shared
  conformance suite. A new endpoint goes in `hairspring-service` first, then `hairspring-server`, then
  an MCP tool if agents need it.
- The API accepts target names only; commands come from the server-side `hairspring.toml`.
- Functions under about 50 lines, files under about 400, early returns, named constants, errors
  handled and never swallowed.
- Behaviour changes update `docs/` and the `Unreleased` section of `CHANGELOG.md`.

## Things not to do

- Do not commit `.env` or `.env.*` files (only `.env.example`), tokens or database files, and never
  print a token or a database URL.
- Do not weaken the server's authentication, rate limits, loopback-only default bind or Host and
  Origin checks.
- Do not add a dependency or an outbound network destination without a decision from a maintainer.
- Do not change `scripts/hermit.pin` or a determinism claim without re-measuring.

## Hard stops and queues

Never push to or merge into `main`, push a tag or publish a release, merge or approve your own pull
request, remove `hold`, `do-not-merge`, `needs-human` or `human-queue`, or expose a secret. Stop and
write a finding before an architecture decision, an auth change, a data-format change existing data
would not survive, or a new dependency. Work from issues labelled `issue-approved` (it requires
acceptance criteria); `human-queue` work is never for agents. A task that cannot be finished ends in
an evidenced finding, never silence. Every commit is signed off (`git commit -s`).

## Before you call a change done

Run what you touched: `cargo fmt --all -- --check`, `make lint`, `cargo test -p <crate>`; in `web/`,
`npm ci && npm run typecheck && npm test && npm run build`; for `scripts/`, `make test-scripts` and `node --test "scripts/tests/*.test.mjs"`. The
required check on a pull request is `ci`. A bare `needs-rework` means make `ci` green and resolve open review comments. When `ci` is red, the `ci-feedback` comment on the pull request ("CI failed on `<sha>`") lists each failing job and its first error lines; read it instead of asking what to change.
