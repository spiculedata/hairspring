## Summary

What changed and why.

Closes #<issue>, if there is one.

## Test plan

How you verified it (commands run, platforms), and what you could not run (a platform, the real Hermit, PostgreSQL,
an SDK toolchain). Say only what you ran.

## Checklist

- [ ] Tests written first; a bug fix includes a test that reproduces the bug
- [ ] `cargo fmt --all` and `make lint` pass
- [ ] `make test` passes (and `npm test` in `web/` if the UI changed)
- [ ] Docs updated where behaviour changed
- [ ] No secrets in code, logs or test fixtures
- [ ] Every commit is signed off (`git commit -s`, see `CONTRIBUTING.md`)
- [ ] The `ci` check is green (read the failing step of `gh pr checks`, not the badge colour)
