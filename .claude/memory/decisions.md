# Decisions

Why the repository and its automation are shaped the way they are, so a later session can tell
when a rule stops applying. Newest first. Product and design decisions live in `SPEC.md`,
`PLAN.md` and `docs/`, not here.

## 2026-11-20: one aggregate `ci` check

`ci.yml` ends in a job named `ci` that needs every other job. A ruleset on `main` can then
require one stable name instead of a list that changes whenever a job is added or a matrix
changes. Two jobs are `workflow_dispatch`-only and are skipped on pull requests, so the
aggregate accepts `skipped` as well as `success`; only `failure` and `cancelled` turn it red.
**Revisit if** a required job could be skipped by a path filter or a failed dependency without
anyone noticing; at that point list the jobs that must be `success` explicitly.

## 2026-11-20: agent state is committed selectively

`.gitignore` ignores `.claude/*` except `settings.json`, `skills` (a symlink to
`.agents/skills`), `memory/` and `session-summary.md`, so the shared, reviewed parts reach every
agent and local scratch state does not. The settings deny force-pushes, history rewrites and
reading env and key files; they ask before merges, releases, tags, publishing, running the
installers and editing workflows, the settings, `AGENTS.md`, `SECURITY.md` and the AI policy.
