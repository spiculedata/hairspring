# GitHub Action

Runs `anthill test`, uploads the findings as code-scanning alerts (SARIF) and attaches the replayable
bundles as an artifact. Build or download `anthill` in an earlier step; the action does not install it.

```yaml
jobs:
  anthill:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write   # for SARIF upload
    steps:
      - uses: actions/checkout@v4
      - run: cargo install --locked --path crates/anthill-cli   # or download a release binary
      - uses: ./integrations/github-action
        with:
          targets: lock-lease pubsub-atomic
          seed: ${{ github.run_number }}
```

Notes
- A failing run uploads `.anthill/bundles/*.anthill`; download one and run `anthill replay <file>` locally to reproduce exactly.
- Pin third-party actions (`github/codeql-action`, `actions/upload-artifact`) to a commit SHA in security-sensitive repositories.
- System-level targets (`kind = "process"`) need Linux x86-64 with Hermit and are best run on a self-hosted runner.
