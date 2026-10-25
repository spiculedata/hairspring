# Contributing

Thanks for helping. Hairspring is small enough that one person can hold the design in their head: keep it that way.

## Setup

```bash
rustup toolchain install stable      # Rust 1.85+ recommended
make ui                              # Node 22+: builds the web UI that the server embeds
make test                            # cargo test --workspace --no-fail-fast
make lint                            # cargo clippy --workspace --all-targets -- -D warnings
make fmt                             # cargo fmt --all (CI runs it with --check)
```

Run one crate's tests with `cargo test -p <crate>`, for example `cargo test -p hairspring-store`.

Web UI tests (Node 22+):

```bash
cd web && npm ci && npm run typecheck && npm test
```

C SDK tests (a C11 compiler and POSIX threads; some tests need Linux, see `sdks/c/Makefile`):

```bash
make -C sdks/c test
```

The Rust SDK (`sdks/rust`) is a workspace member: `cargo test -p hairspring-sdk` runs its tests, and
`cargo test -p hairspring-props --test rust_sdk` runs the conformance tests against the reference parser (they build
the SDK's sample programs with a nested `cargo build`, and compare with the C SDK, which needs `make` and a C compiler).
Its seccomp "no system call" check runs on Linux only.

The Python SDK (`sdks/python`, Python 3.12 or newer, standard library only) has its own tests, run from the repository
root with `python3 -m unittest discover -s sdks/python/tests -t sdks/python`. Its conformance tests against the
reference parser are `cargo test -p hairspring-props --test python_sdk` (they skip with a notice when there is no
Python 3.12+; `HS_REQUIRE_PYTHON=1` makes that an error, `HS_PYTHON` names the interpreter), and the lost-update
fixture is checked with `make -C examples/language-fixtures/python test`.

The .NET SDK (`sdks/dotnet`, the .NET 10 SDK, base class library only) has its own tests: `make -C sdks/dotnet test` runs the
self-test and the sample programs as real processes. Its conformance tests against the reference parser are
`cargo test -p hairspring-props --test dotnet_sdk` (they skip with a notice when there is no .NET SDK 10 or newer;
`HS_REQUIRE_DOTNET=1` makes that an error, `HS_DOTNET` names the `dotnet` command), and the lost-update fixture is checked with
`make -C examples/language-fixtures/dotnet at-test`.

PostgreSQL tests are skipped unless `HAIRSPRING_TEST_PG_URL` is set. Besides the store conformance suite, the
service and CLI tests also run against PostgreSQL when it is set:

```bash
docker run -d --name pg -e POSTGRES_PASSWORD=x -e POSTGRES_DB=hairspring -p 127.0.0.1:54329:5432 postgres:17-alpine
export HAIRSPRING_TEST_PG_URL=postgres://postgres:x@127.0.0.1:54329/hairspring
cargo test -p hairspring-store --features postgres
cargo test -p hairspring-service -p hairspring-cli
```

Tests against the real Hermit need x86-64 Linux with performance counters:
`HAIRSPRING_HERMIT=/path/to/hermit HAIRSPRING_HERMIT_REAL=1 cargo test -p hairspring-hermit --test real_hermit`.
CI does not run these.

## How we work

- **Tests first.** Write the test, watch it fail for the right reason, then make it pass. Bug fixes start with a test that reproduces the bug.
- **Say what you verified.** A claim in docs or a PR ("works on Linux", "deterministic") should name how it was checked.
- **Small, immutable-leaning code.** Functions under ~50 lines, files under ~400 (800 hard limit), early returns, named constants, no mutation where a new value is as clear. Handle errors explicitly; never swallow them.
- **No secrets in the repo, logs or error messages.** Treat anything from a worker, a target or a network client as untrusted.
- **Dependencies are a cost.** `hairspring-core` is std-only on purpose. Before adding a crate or npm package, check it is maintained, has no known advisories (`cargo audit`), and that the standard library cannot do the job in a few lines. The dependency list in `NOTICE` is maintained by hand: update it when you add or remove a direct dependency, and treat `cargo metadata --locked` as authoritative.
- **Commits:** `type: description` with types `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`.

## Where things go

| You want to... | Look at |
|---|---|
| Add a new kind of target or execution engine | Implement `hairspring_engine::backend::Backend` (see `hairspring-wasm` for a compact example), register it in `hairspring-cli/src/commands/mod.rs` |
| Change what is stored | `hairspring-store`: update **both** `sqlite.rs` and `pg.rs`, and extend `tests/common/mod.rs` (the conformance suite both must pass) |
| Add an API endpoint | `hairspring-service` first (the logic and its tests), then `hairspring-server`, then the MCP tool if agents need it |
| Change the UI | `web/` (`npm run dev` proxies to `hairspring serve`); keep untrusted text rendered as text |

## Pull requests

CI runs fmt, clippy, tests on Linux and macOS, the UI typecheck, tests and build, and dependency audits (`make audit`
runs the same audits locally). Use a conventional-commit title (`docs: ...`, `fix: ...`). Describe the problem,
the approach, and how you tested it; the pull request template has a checklist. Security-sensitive changes (auth, input handling, process execution) get extra review.
