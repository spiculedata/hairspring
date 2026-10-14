# Contributing

Thanks for helping. Anthill is small enough that one person can hold the design in their head: keep it that way.

## Setup

```bash
rustup toolchain install stable      # Rust 1.85+ recommended
make ui                              # Node 22+: builds the web UI that the server embeds
make test                            # cargo test --workspace --no-fail-fast
make lint                            # clippy with warnings as errors
```

PostgreSQL tests are skipped unless you provide a database:

```bash
docker run -d --name pg -e POSTGRES_PASSWORD=x -e POSTGRES_DB=anthill -p 127.0.0.1:54329:5432 postgres:17-alpine
ANTHILL_TEST_PG_URL=postgres://postgres:x@127.0.0.1:54329/anthill cargo test -p anthill-store --features postgres
```

Tests against the real Hermit need x86-64 Linux with performance counters:
`ANTHILL_HERMIT=/path/to/hermit ANTHILL_HERMIT_REAL=1 cargo test -p anthill-hermit --test real_hermit`.

## How we work

- **Tests first.** Write the test, watch it fail for the right reason, then make it pass. Bug fixes start with a test that reproduces the bug.
- **Say what you verified.** A claim in docs or a PR ("works on Linux", "deterministic") should name how it was checked.
- **Small, immutable-leaning code.** Functions under ~50 lines, files under ~400 (800 hard limit), early returns, named constants, no mutation where a new value is as clear. Handle errors explicitly; never swallow them.
- **No secrets in the repo, logs or error messages.** Treat anything from a worker, a target or a network client as untrusted.
- **Dependencies are a cost.** `anthill-core` is std-only on purpose. Before adding a crate or npm package, check it is maintained, has no known advisories (`cargo audit`), and that the standard library cannot do the job in a few lines.
- **Commits:** `type: description` with types `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`.

## Where things go

| You want to... | Look at |
|---|---|
| Add a new kind of target or execution engine | Implement `anthill_engine::backend::Backend` (see `anthill-wasm` for a compact example), register it in `anthill-cli/src/commands/mod.rs` |
| Change what is stored | `anthill-store`: update **both** `sqlite.rs` and `pg.rs`, and extend `tests/common/mod.rs` (the conformance suite both must pass) |
| Add an API endpoint | `anthill-service` first (the logic and its tests), then `anthill-server`, then the MCP tool if agents need it |
| Change the UI | `web/` (`npm run dev` proxies to `anthill serve`); keep untrusted text rendered as text |

## Pull requests

CI runs fmt, clippy, tests on Linux and macOS, the UI build and tests, and dependency audits. Describe the problem,
the approach, and how you tested it. Security-sensitive changes (auth, input handling, process execution) get extra review.
