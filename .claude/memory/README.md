# Correction capture

An append-only record of corrections: things a human or CI told an agent that turned out
to matter, so the next session does not have to be told again.

`corrections.jsonl` at the repo root is the log. One JSON object per line; never edited,
reordered or deleted. A correction that stopped being true gets a new entry that
supersedes it.

```json
{"id":"corr-0001","date":"2026-11-20","scope":"repo","signal":"ci","area":"workflow","correction":"What was wrong, stated plainly.","rule":"The durable instruction.","evidence":"PR, file:line or command","supersedes":null,"status":"active"}
```

- `status` is `active` or `promoted`. Promoting a lesson means writing the timeless rule into
  `AGENTS.md` or a skill, then appending a new entry with `"status":"promoted"`, `"supersedes"`
  naming the old id and `"promoted_to"` (e.g. `AGENTS.md#rules`). The old line stays as it was.
- `signal`: `human`, `ci` or `review`.
- `rule` is the instruction, not the story. The story belongs in a doc if it is worth keeping.
- Keep it small. A log nobody reads looks like coverage and is worse than none.

Related: `.claude/memory/decisions.md` (why the repository is shaped this way),
`.claude/session-summary.md` (where the work stands).
