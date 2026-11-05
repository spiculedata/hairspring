# GitHub Action

Runs `hairspring test`, uploads the findings as code-scanning alerts (SARIF) and attaches the replayable
bundles as an artifact. By default the action installs the CLI itself from a release binary.

This has not been run on GitHub-hosted runners by the Hairspring maintainers; the examples below describe how the
action is written to behave. The binary download and checksum steps are the part most worth watching on a first run.

## Use it from another repository

Pin the action by full commit SHA, not by a tag or branch:

```yaml
jobs:
  hairspring:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write   # for SARIF upload
    steps:
      - uses: actions/checkout@v4
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: lock-lease pubsub-atomic
          seed: ${{ github.run_number }}
```

Your repository needs a `hairspring.toml` (or pass `config:`).

## How the CLI is installed

- Default: the action downloads the release binary for the runner (`hairspring-linux-x86_64` on Linux x64,
  `hairspring-macos-arm64` on macOS arm64) from
  <https://github.com/spiculedata/hairspring/releases>, checks its sha256 against the release's `SHA256SUMS` and
  stops with an error if they differ. Other runners get an error that points at the options below.
- `version`: the release tag to download. The default is the newest pre-release the action was tested with
  (`v0.1-RC1`). Pin the action commit and the version together.
- `from-source: true`: build the CLI with `cargo` from the action's own checkout instead. Needs a Rust toolchain on
  the runner and takes longer.
- `hairspring-bin`: a path to a binary you installed in an earlier step. When set, nothing is downloaded or built.

## One run, one report

`hairspring test` takes a single `--format`, so the action makes one run and writes one report. Set `format` to
`sarif` (the default; the only format that is uploaded as code-scanning alerts), `junit` or `text`. The report is
written to `hairspring.sarif`, `hairspring-junit.xml` or `hairspring.txt` and attached to the `hairspring-results`
artifact. If you want both SARIF and JUnit, call the action twice with the same `seed`; each call runs the tests
again.

With an empty `seed` the run picks a random seed and prints it in the log.

## Regression sweep

A sweep runs many seeds instead of stopping at the first failure. `seeds` is `FIRST..END` with END left out, so
`100..200` is 100 trials:

```yaml
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: lock-lease
          sweep: "true"
          seeds: 100..200
```

## Process targets on a self-hosted runner

Targets with `kind = "process"` need Linux x86-64 with Hermit and PMU access. Use a self-hosted bare-metal runner
and install Hermit on it yourself. GitHub-hosted runners were not tested for process targets.

```yaml
jobs:
  hairspring-process:
    runs-on: [self-hosted, linux, x64]
    permissions:
      contents: read
      security-events: write
    steps:
      - uses: actions/checkout@v4
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: my-process-target
          config: hairspring.toml
```

## Inputs

| Input | Default | Meaning |
|---|---|---|
| `targets` | all in the config | Space-separated target names |
| `seed` | random | Exploration seed; a sweep runs consecutive seeds from it |
| `config` | `hairspring.toml` | Path to the configuration |
| `format` | `sarif` | `sarif`, `junit` or `text` |
| `sweep` | `false` | Run a sweep |
| `seeds` | empty | Sweep seed range, `FIRST..END` |
| `version` | `v0.1-RC1` | Release tag to download |
| `from-source` | `false` | Build with cargo instead of downloading |
| `hairspring-bin` | empty | Use this binary; skip install |
| `upload-sarif` | `true` | Upload SARIF (only when `format` is `sarif`) |

Output: `exit-code` (0 = all properties held, 1 = a violation was found, 2 = error).

## Notes

- A failing run uploads `.hairspring/bundles/*.hairspring`; download one and run `hairspring replay <file>` locally to
  reproduce exactly.
- Pin third-party actions (`github/codeql-action`, `actions/upload-artifact`) to a commit SHA in security-sensitive
  repositories.
- This repository's own CI does not use the action, so the action is checked with `actionlint` and by reading, not
  by a self-test job.
