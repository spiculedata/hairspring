# AI workflows

One GitHub Actions workflow lets an AI agent act on this repository when a maintainer asks it to; a second
exists only as a disabled stub. The policy they sit under is `docs/security/SECURITY-AI.md`. The repository
is public, so the first rule is that issue and pull request text is written by strangers and is never trusted.

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `ai-fix.yml` | A maintainer adds the **`ai-fix-requested`** label to an issue | Attempts the fix on a new `ai-fix/...` branch and opens a pull request that says `Fixes #N` |
| `auto-review.yml` | The **`apply-review`** label on a pull request | **Disabled.** The file is a stub with no checkout, no install, no agent step and no write scope (see below) |

## Queues and hard stops

Agents work only from issues labelled `issue-approved`, which requires acceptance criteria
(`.github/workflows/issue-gate.yml` removes the label and comments when they are missing). `human-queue`,
`hold`, `do-not-merge`, `needs-human`, `blocked`, `wontfix` and `duplicate` are never agent work. The hard
stops and the "stop and write a finding" list are in `AGENTS.md`; `ai-fix` is bounded by them as well as by
its tool allowlist. `ai-fix-requested` is a separate, maintainer-only trigger.

## Turning ai-fix on

1. Add one repository secret: `CLAUDE_CODE_OAUTH_TOKEN` (create it with `claude setup-token`) or
   `ANTHROPIC_API_KEY`. Settings, Secrets and variables, Actions. Without it the workflow logs "not
   configured" and stops; merging the file changes nothing by itself.
2. The Claude GitHub App must be installed on the repository, which the action uses for its token exchange.
   Whether it is installed here has not been checked.
3. Decide on `AI_FIX_ALLOWED_BOTS` (a repository variable, space-separated logins, empty by default). The
   Hive's bot, `spicule-hive-bot`, is an ordinary user with write access; it is treated as automated and
   is **not** allowed unless listed, so an issue the Hive files with the label does not start a paid run by itself.
4. Know what is enforced. Which rules protect `main` is a repository setting: check
   `gh api repos/spiculedata/hairspring/rules/branches/main`. `ai-fix` holds `contents: write`, so "never
   pushes to `main`" also rests on the prompt, the tool allowlist (`git push` is limited to the agent's own
   branch and `git push origin main` is denied) and a human reviewing every pull request.
   `main-guard.yml` opens an issue when a commit reaches `main` without a pull request; it detects, it cannot prevent.

## What keeps ai-fix bounded

- **Only maintainers start it.** The label must be added by a user with write access (`admin`, `maintain` or
  `write`); a user who can only triage cannot. A bot (type Bot, or a login ending in `[bot]` or `-bot`) can
  start it only if listed in `AI_FIX_ALLOWED_BOTS`.
- **Untrusted text is data.** The prompt tells the agent that the issue may not change its rules, and tools are an
  allowlist: edit files, `cargo fmt`, `clippy`, `test` and `check`, the web UI scripts, the Node script tests,
  `git add` and `commit`, pushing its own `ai-fix/*` branch, and opening a PR or comment. No curl, no
  arbitrary shell, force-pushes denied.
- **Bounded runs.** 30 turns and 30 minutes, one run per issue at a time.
- **Scope limits.** Edits are denied (by `--disallowedTools`) to `.github/`, `.claude/`, `docs/security/`,
  `SECURITY.md`, `AGENTS.md`, `CLAUDE.md`, `DCO`, `LICENSE`, `Dockerfile`, `docker-compose.yml`, the installers,
  `scripts/hermit.pin`, the GitHub Action, and the critical-tier Rust files (server auth and limits, the config
  reader, the Hermit backend, the two store schemas, the bundle format). A fix that needs one of them ends as a
  finding on the issue. `.github/risk-tiers.yml` classifies such a change as `critical` if it happens anyway.
- **No pull request code.** `ai-fix` runs on `issues`, from the default branch. Neither workflow uses
  `pull_request_target`, and a fork's pull request never reaches the secret.
- **Sign-off.** The prompt tells the agent to commit with `git commit -s` (this repository requires the DCO).
- **A human still decides.** Everything arrives as a pull request. `ci` must pass and a reviewer must merge.

## Why auto-review is disabled

`auto-review` was designed to apply review comments to a pull request's own branch. Doing that means checking
out the branch with write credentials and installing its dependencies, which runs PR-controlled lifecycle and
build scripts on a runner holding a write token and the AI credential; kinetic disabled its copy for that reason.
This repository is public and runs Rust build scripts (`build.rs`) in `cargo test`, so the risk is larger here.
It is a stub until someone designs and reviews a version that keeps pull request code away from both credentials.

## Known limits

- A pull request or push made with the default token does not trigger `ci` on its own: GitHub creates the runs
  and holds them as `action_required` until a maintainer approves them in the Actions tab. `ai-fix` stays on the
  default token on purpose, so a human starts CI on its pull requests. Nothing in this repository approves runs.
- `ai-fix` can run `cargo test` and `npm test`, which execute repository code with the job's token in the
  environment. That is the trust the maintainer label grants.
- `ai-fix` has never run: it is inert until the secret exists. It has been syntax-checked with actionlint and its
  guard tests run (`scripts/tests/ai-workflows.test.mjs`); the agent steps themselves have not run.
- The agent can be wrong. The label saves typing, not review.

## Turning it off

Delete the secret (the workflow goes quiet) or delete the workflow file.
