---
name: hairspring-check
description: Run Hairspring's pre-PR gate for the parts a change touched (cargo fmt, clippy, tests, the web UI checks, script tests, audits) and report what actually ran. Use before opening or updating a pull request, or when asked whether a change is done.
---

# hairspring-check

Run from the repository root. Report the real output of every step; do not summarise a step you did
not run, and say which you could not run and why. These are the same commands CI runs
(`.github/workflows/ci.yml`); the check that must be green on the pull request is the aggregate job `ci`.

1. See what changed: `git diff --stat main...HEAD` (or `git status` before committing). It decides
   which of the steps below apply.
2. **Rust changed** (anything under `crates/`, `sdks/rust/`, `experiments/` or a `Cargo.toml` or
   `Cargo.lock`):
   - `cargo fmt --all -- --check`
   - `make lint` (`cargo clippy --workspace --all-targets -- -D warnings`)
   - `cargo test -p <crate>` for each crate you touched and each crate that depends on it. Use
     `make test` (`cargo test --workspace --no-fail-fast`) when you changed `hairspring-core`, the
     engine, the store or the service. The workspace run includes SDK conformance tests that print a
     skip notice when Python, Java or .NET is missing; say so rather than calling them passed.
   - A store change: also the PostgreSQL run in `CONTRIBUTING.md` (needs Docker), or say you did not.
3. **`web/` changed**: `cd web && npm ci && npm run typecheck && npm test && npm run build`.
4. **`scripts/` or the installers changed**: `make test-scripts` (needs `shellcheck`). Never run
   `scripts/install.sh` or `scripts/install-hermit.sh` for real; the tests use fake commands.
5. **An SDK changed** (`sdks/<lang>/`): that SDK's own tests (`make -C sdks/c test`, `go test -race -count=1 ./...`
   in `sdks/go`, `make -C sdks/java test`, `make -C sdks/dotnet test`,
   `python3 -m unittest discover -s sdks/python/tests -t sdks/python`) and the matching
   `cargo test -p hairspring-props --test <go|java|python|dotnet|rust|c>_sdk`.
6. **Dependencies changed**: `make audit` (`cargo audit`, and `npm audit --omit=dev` in `web/`), and
   update the dependency list in `NOTICE`. Do not exclude an advisory to make it pass.
7. **Workflows, `.github/risk-tiers.yml` or the `.mjs` helpers changed**: `actionlint` if installed,
   `bash scripts/check-workflow-shells.sh`, and `node --test "scripts/tests/*.test.mjs"` (the workflow
   guards and the classifier tests; CI runs them in the `Web UI` job).
8. Docs and changelog: a behaviour change updates `docs/` and the `Unreleased` section of `CHANGELOG.md`.
9. Commits: every commit is signed off (`git log --format='%h %s%n%b' main..HEAD | grep -c Signed-off-by`
   should equal the number of commits) and is titled `type: description`.

## Reporting

For each step say: ran and passed, ran and failed (with the first error line), or not run (and why).
Tests against the real Hermit (`HAIRSPRING_HERMIT_REAL=1`) need x86-64 Linux with performance counters,
and CI does not run them: never call them passed unless you ran them. When a pull request is red, read the
failing *step* (`gh pr checks <n>`, then the run log), not the colour of the badge.
