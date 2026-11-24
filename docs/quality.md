# Quality dashboard

One page that answers "how healthy is this codebase right now?". `ci` is the gate; this is the
wider picture, with numbers that come from the tools and nowhere else.

## Where to find it

Run **quality-report** from the Actions tab (it also runs weekly on Mondays). Each run publishes the
page in its job summary and as the `quality-report` artifact. Nothing is committed, so the numbers can
never go stale in the repository.

## What is measured

| Signal | Tool | Command it runs |
| --- | --- | --- |
| Rust tests | cargo | `cargo test --workspace --no-fail-fast` (output summed over every `test result:` line) |
| Rust lint | clippy | `cargo clippy --workspace --all-targets --message-format=short` (diagnostics counted) |
| Rust audit | cargo-audit | `cargo audit --json` (vulnerabilities, and the unmaintained, unsound and yanked warnings) |
| Web tests | Vitest | `vitest run --reporter=json` in `web/` |
| Web audit (production) | npm | `npm audit --omit=dev --json` (the CI gate runs the same scope) |
| Web audit (all, incl. dev) | npm | `npm audit --json` (informational) |
| Web coverage | Vitest + v8 | `vitest run --coverage` with the `json-summary` reporter, provider installed with `--no-save` |

The Rust tools run on the weekly and manual runs only, because they take as long as the Rust job in
`ci.yml`. On a pull request that changes the report itself they are skipped, and the page shows them as
"not available". Rust **coverage is not measured**: no coverage tool is wired for the Rust workspace, and
the page says so instead of showing a number.

A signal whose tool did not produce output is shown as "not available". It is never shown as zero.

## Reading it

- **Needs attention** appears when a Rust test fails, clippy reports a diagnostic, `cargo audit` finds a
  vulnerability, a web test fails, or the web production audit has HIGH or CRITICAL advisories. These are
  the conditions `ci` fails on, so it should be rare.
- **Report incomplete** lists the signals that could not be read.
- The dev-dependency audit and the cargo-audit warnings (unmaintained, unsound, yanked) are informational.

## Coverage gate

`.github/workflows/coverage-gate.yml` (named `Coverage gate`) fails a pull request when web coverage
or Rust workspace line coverage drops below `.coverage-thresholds.json`. The thresholds are a ratchet set a few
points below what was measured: web on 2026-11-20 (statements 92.23, branches 87.65, functions 90.32, lines 93.6
against 88, 83, 86 and 89) and the Rust workspace with `cargo llvm-cov --workspace --no-fail-fast --json
--summary-only` on 2026-11-23 (lines 87.40 against a floor of 85; the same run's per-crate numbers are printed to
the job summary, and `hairspring-store` is the low outlier at 60% because the PostgreSQL tests are skipped when
`HAIRSPRING_TEST_PG_URL` is unset, as in the gate): raise them as coverage grows, never lower them. The checker
validates each configured threshold before comparing it with a report (a finite number greater than 0 and at most
100) and fails with exit 2 on a malformed one, so a typo such as `null` cannot silently disable a metric's floor.
The scripts are reported, not gated.

## Changing it

The renderer is `scripts/quality-report.mjs`, with tests in `scripts/tests/quality-report.test.mjs` (they
also check that the workflow writes every file the script reads). Add a signal by saving the tool's output in
the workflow and teaching the script to summarise it.
