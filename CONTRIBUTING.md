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

After a release build (`make release`, which runs it for you), `make check-ui` checks that the binary serves the UI built
in `web/dist` and not an older one. It is for release builds only: a debug build reads `web/dist` from disk.

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
service and CLI tests also run against PostgreSQL when it is set. The CI job "PostgreSQL tests" runs them against a
`postgres:16.6-alpine` service container with trust authentication (one store test creates a login role without a
password and connects as it) and sets `HAIRSPRING_REQUIRE_PG=1`, so a missing database fails the tests instead of
skipping them. To run the same thing locally:

```bash
docker run -d --name pg -e POSTGRES_HOST_AUTH_METHOD=trust -e POSTGRES_DB=hairspring -p 127.0.0.1:54329:5432 postgres:16.6-alpine
export HAIRSPRING_TEST_PG_URL=postgres://postgres@127.0.0.1:54329/hairspring
export HAIRSPRING_REQUIRE_PG=1
cargo test -p hairspring-store -p hairspring-service -p hairspring-cli --features hairspring-store/postgres --no-fail-fast
```

Tests against the real Hermit need x86-64 Linux with performance counters:
`HAIRSPRING_HERMIT=/path/to/hermit HAIRSPRING_HERMIT_REAL=1 cargo test -p hairspring-hermit --test real_hermit`.
CI does not run these.

### Checking what changed in upstream Hermit

The Hermit build is pinned in `scripts/hermit.pin`: a fork, one commit, a replay patch checked by sha256 and a dated Rust
toolchain. Everything we say about which languages work under Hermit was measured with that build, so moving the pin
means measuring again. Check what changed upstream before deciding to bump the pin:

```bash
bash scripts/hermit-upstream-report.sh > hermit-upstream.md      # needs git and network access; gh is optional
bash scripts/hermit-upstream-report.sh --no-issues --limit 50    # skip the issue search, list more commits per group
bash scripts/hermit-upstream-report.sh --self-test               # offline check against a local fixture (CI runs it)
```

The report also runs weekly in GitHub Actions (`.github/workflows/hermit-upstream.yml`, Mondays at 06:23 UTC, or on demand
from the Actions tab with "Run workflow"). Open the run and read the job summary, or download the
`hermit-upstream-report` artifact (kept for 30 days). The run is read-only: it does not change the repository or open
issues or pull requests, and it is separate from the CI checks, so it never fails a pull request. It goes red only if the
script itself fails. Scheduled runs start from the default branch, so a change to the workflow takes effect once merged.

The self-test prints one `ok` line per assertion and ends with `self-test passed: N assertions`; any `FAIL` line fails it.

The report is Markdown. It gives the pinned commit and the upstream commit it is based on, how far the pin and the
fork are behind (or ahead of) the upstream, whether the replay patch still applies to the upstream head (`git apply
--check` only: nothing is built or run), the Rust toolchain on each side, the newest version tag, the new upstream
commits grouped by keywords in their subjects (fix, feat, perf, build, deps), a watch list of commits in areas the
Hermit backend depends on (preemption and skid, timeouts, temporary files, the summary, CPUID and time, `/proc`), open
upstream issues by title keyword (through `gh`, when it is installed and signed in) and the last-activity dates. The
groups come from commit subjects alone, so read a commit before relying on its group.

The script is read-only. It works in a temporary directory that it deletes on exit and never changes the repository,
the pin or the patch. It parses `scripts/hermit.pin` strictly and never sources it, so a new key in the pin needs the
script updated too. A bump is a separate change, and it needs the Hermit-backed results re-run on the new build.

## Licence of contributions and sign-off

Hairspring is licensed under the Apache License, Version 2.0 (see `LICENSE`). By submitting a pull request you agree
that your contribution is licensed under the same terms, as set out in section 5 of that licence.

We use the [Developer Certificate of Origin](https://developercertificate.org/) (the text is in `DCO`) instead of a
contributor licence agreement. It is your statement that you wrote the change or have the right to submit it. Say so by
adding a `Signed-off-by` line, with your real name and an email address you can be reached at, to every commit:

```bash
git commit -s -m "fix: describe the change"
```

This adds `Signed-off-by: Your Name <you@example.com>` using your `user.name` and `user.email`. To add it to commits
you have already made on a branch, run `git rebase --signoff main` (or `git commit --amend -s` for the last one) and
push again. Commits made on your behalf by a tool are signed off by you, not by the tool. If you cannot sign off a
change, say why in the pull request.

Sign-off is checked by a maintainer at review time. There is no automated check yet.

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
