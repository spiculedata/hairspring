# Session summary

Where the work stands, rewritten by whoever last worked on it. Durable knowledge lives in
`AGENTS.md` (invariants), `corrections.jsonl` (lessons) and `.claude/memory/decisions.md` (why).
Rewritten, never appended; delete a line when it stops being true.

_Last updated: 2026-11-20_

## Where we are

- The repository is being brought in line with the shared agent-ready template used by the other
  spiculedata repositories, in three pull requests: the stable `ci` check with the agent
  instructions, then the agent safety settings and AI policy, then the workflows and their scripts.
- Product work continues in `CHANGELOG.md` (`Unreleased`); this file does not track it.

## Decisions pending

- Whether to require `ci` through a ruleset on `main` once the first pull request has merged and
  `ci` has run green on `main` (a maintainer's decision, not a file in this repository).

## Unverified

- That the `ci` aggregate job is green on GitHub: it was checked with actionlint and a local
  simulation only until the first pull request ran it.

## Next

1. Merge the three pull requests in order and retarget the later ones to `main`.
2. Confirm the check list the ruleset requires (`ci`) with
   `gh api repos/spiculedata/hairspring/rules/branches/main`.
