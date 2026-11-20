# Acceptance specs

An issue's acceptance criteria become `acceptance/<issue>/spec.json`, committed with the pull request that
closes it, so "done" is something a command can check. The format, the runner and the CI workflows are the
shared, public [`spiculedata/acceptance-spec`](https://github.com/spiculedata/acceptance-spec) repository:
[writing and running a spec](https://github.com/spiculedata/acceptance-spec#writing-and-running-a-spec) and
[every field of `spec.json`](https://github.com/spiculedata/acceptance-spec/blob/main/docs/spec-format.md).
This directory holds no specs yet.

## Status here: convention documented, enforcement not wired

Read this before assuming a spec is checked on a pull request.

- **Wired, report only.** `.github/workflows/thin-caller-gate.yml` warns when this wiring drifts (a caller not
  pinned to a full commit SHA, callers on different SHAs, a local copy of a shared script). It runs on pull
  requests in report mode, is not part of `ci`, and blocks nothing. `.github/workflows/acceptance-pin-health.yml`
  reports on Mondays and on manual dispatch how far the pinned SHA is behind `acceptance-spec` `main`; it
  opens no issue.
- **Not wired.** The three shared workflows that would make specs part of `ci` (`convention`, `run-specs` and
  `base-check`) are **not called** from `ci.yml`. Adding them needs two decisions that belong to a maintainer:
  the cutoff issue number (`enforce-from-issue`: only issues numbered at or above it owe a spec; take the
  newest issue or PR number plus one), and how a run builds and starts the app. Hairspring is a Rust server
  with a web UI embedded in the binary (`make ui`, then `cargo build`), so the spec kind to use is
  `command` (the exit code is the verdict; `api` specs need a `base-url`). Until that is decided, a spec in
  this directory is written and run by hand, and nothing in CI fails for a missing or red spec.

## Writing one by hand

```bash
git clone https://github.com/spiculedata/acceptance-spec ~/acceptance-spec
git -C ~/acceptance-spec checkout e1ee02261986f510ca3be97f09b07546dd0614be   # the SHA pinned in the two workflows
node ~/acceptance-spec/scripts/acceptance-runner.mjs run --dir acceptance/<issue>
```

It prints `pass`, `fail` or `not-run` and exits 1 on a failure. Commit the spec before the fix and check
that it fails on `main`; a spec that already passes proves nothing.

## The pin

Both workflows name the same full 40-character SHA, with a `# v1 acceptance-spec@<short sha>` comment. Change
them together, in one pull request: `thin-caller-gate` reports callers on different SHAs, and
`scripts/tests/acceptance-wiring.test.mjs` fails if they differ or are not a full SHA.

## Labels

`acceptance-waived` and `spec-already-green` exist in the repository. Both are applied by a human only; an
agent never applies either. `acceptance-waived` excuses the convention for a docs-only or tests-only change;
`spec-already-green` excuses a spec that legitimately passes on `main` already. Neither has any effect until
the three shared workflows are wired.
