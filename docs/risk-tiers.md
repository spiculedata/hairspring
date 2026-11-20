# Risk tiers

What each change to Hairspring demands, decided once instead of argued per pull request. The rule is
[`.github/risk-tiers.yml`](../.github/risk-tiers.yml); it is applied by
[`scripts/classify-change.mjs`](../scripts/classify-change.mjs), which
[`.github/workflows/tier-classifier.yml`](../.github/workflows/tier-classifier.yml) runs on every pull
request. This page is the reasoning, the YAML is the rule.

## What a tier is

A tier is a claim about how bad a mistake would be, not how hard the change is. A 400-line refactor of
the explorer is `high` at worst. A two-line change to the bearer-token check, the schema or the installer
is `critical`. The classifier labels the pull request (`risk/<tier>`) and keeps one comment up to date
naming the tier, the review it asks for and the files that set it. **It is advisory.** It is not a
required check and cannot block a merge; the check that must be green is `ci`.

## The tiers

| Tier | Label | Covers | Review |
| --- | --- | --- | --- |
| `critical` | `risk/critical` | Server authentication, rate limits, connection and body limits, client-address and token-label handling, the worker and confirm endpoints, the config that names the commands a process target runs, the whole Hermit backend, the SQLite and PostgreSQL schemas, the bundle format, the installers and `scripts/hermit.pin`, the GitHub Action, `Dockerfile` and `docker-compose.yml`, `.env.example`, `SECURITY.md`, `docs/security/`, `DCO`, `LICENSE`, all workflows, agent instructions and settings | A named human, with the PR body saying what was checked and how, and what was not run. |
| `high` | `risk/high` | The rest of `crates/`, the SDKs, `Cargo.toml` and `Cargo.lock`, the web manifests and lockfile, `global.json`, `NOTICE`, `scripts/`, the tuning and coverage files, the Copilot and Cursor copies of the agent instructions | A human reads the diff; the PR body names the tests and what could not run here. |
| `standard` | `risk/standard` | `web/`, `examples/`, `experiments/`, `integrations/`, `site/`, the `Makefile`, and anything that matches nothing else | Normal review, the checks for what changed green. |
| `low` | `risk/low` | `docs/`, markdown, issue and PR templates, `.gitignore`, `corrections.jsonl` | A skim: is the prose still true? |

A pull request takes the highest tier any one file matched, so forty doc files do not dilute one change
to the auth check. Files that match nothing take `default_tier` (`standard`), which is a real tier with a
real review requirement.

## Why these paths are critical

Hairspring runs code you point it at and listens on a network port (`SECURITY.md`), so the critical tier
is about who may call the server and what it may run:

- **Authentication and limits** (`auth.rs`, `ratelimit.rs`, `limits.rs`, `conn.rs`, `client_addr.rs`,
  `token_label.rs`): a regression is silent and shows up as an exposure, not a test failure.
- **What can be run**: `hairspring.toml` names the commands process targets execute (`config.rs`), the
  Hermit backend executes them (`crates/hairspring-hermit/src`), and `confirm_api.rs` and `worker_api.rs`
  are the endpoints that cause or accept the result of an execution.
- **Data that cannot be repaired**: the two store schemas run against every user's database, and the
  bundle format decides whether an existing bundle still replays.
- **Code that downloads and runs binaries**: `scripts/install.sh`, `scripts/install-hermit.sh`, the pinned
  build in `scripts/hermit.pin`, and the composite GitHub Action other repositories run in their CI.
- **Gates**: `.github/workflows/**` (including `release.yml`, which builds the draft release),
  `.claude/settings.json`, the instruction files and the AI policy decide what the next change may skip.

## Input is paths, nothing else

The classifier reads the list of changed file paths from the API. It never reads the pull request title,
body, branch name or comments, and never calls a model. Same files, same answer; there is nothing in it to
prompt-inject. Patterns are globs: `*` stops at a slash, `**` does not.

## Changing a tier

Edit `.github/risk-tiers.yml` and run `node --test "scripts/tests/*.test.mjs"` (CI runs the same command in
the `Web UI` job). The tests fail if a tier loses its summary or review text, if two tiers share a label,
or if a listed pattern matches no file in the repository (a renamed file would otherwise be silently
downgraded to the default tier). A pattern for a file that does not exist on this branch yet is marked
`pending: true`; remove the flag once the file lands. A `why` that contains `: ` must be quoted.

## Labels

`risk/critical`, `risk/high`, `risk/standard` and `risk/low` exist in the repository. The workflow also
tries to create them if they are missing and carries on if it is not allowed to.
