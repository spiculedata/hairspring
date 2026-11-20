# Review a pull request

## Prompt

```
Review pull request #<number> in spiculedata/hairspring.

Read AGENTS.md first, then docs/security/SECURITY-AI.md and SECURITY.md. Gather what a reviewer needs:

    gh pr view <number> --json title,body,files,additions,deletions
    gh pr checks <number>
    gh pr diff <number>

Read the description before the diff. The "why" and the test plan are part of the change under
review: a PR that says it touched nothing security-relevant while the diff touches the server's
auth, limits, the config reader, the Hermit backend, a schema, the bundle format, an installer or a
workflow is a finding, and a blocking one (these are the `critical` paths in .github/risk-tiers.yml).

If CI is red, say which job failed (read the failing step, not the badge) and stop. Do not review a
diff that is about to be re-run.

Work the blocking rules in order and quote file:line for each. A rule you cannot point at a line for
is "unchecked", which is different from "passed".

  1. Secrets: no .env*, tokens, database URLs or key files in the diff, logs, test fixtures or error
     messages.
  2. Untrusted input: text from a target, a guest, a worker or a bundle is stored and rendered as
     text, never executed or interpolated into a command; the API accepts target names only; limits
     and role checks are not weakened.
  3. Layers and storage: front ends depend on the service, the service on the engine and store; a
     store change updates both sqlite.rs and pg.rs and the shared conformance suite; hairspring-core
     keeps its single dependency.
  4. Determinism and measurement claims: a docs or README claim ("works on Linux", "deterministic",
     "N times faster") names how it was measured; scripts/hermit.pin changes come with re-measured
     results.
  5. Docs and changelog: behaviour changes update docs/ and CHANGELOG.md (Unreleased); NOTICE is
     updated when a direct dependency changes.
  6. Sign-off and message: every commit has a Signed-off-by line and a `type: description` title.

Then tests: new behaviour has a test; a bug fix has a test that fails without the fix.

Severity: blocking, should fix, nit. Never promote a nit. Never demote anything that weakens
authentication, limits or process execution.

Do not report what CI already settles (fmt, clippy, the tests, the audits). Say what you could not
verify, and why (a platform, the real Hermit, PostgreSQL, a language toolchain).
```

## Output shape

A verdict (approve, request changes, or comment), then findings grouped by severity, each with
file:line and a one-sentence fix. End with what was not verified.
