# GitHub Action

Runs `hairspring test`, uploads the findings as code-scanning alerts (SARIF) and attaches the replayable
bundles as an artifact. By default the action installs the CLI itself from a release binary.

This has not been run on GitHub-hosted runners by the Hairspring maintainers; the examples below describe how the
action is written to behave. The binary download and checksum steps are the part most worth watching on a first run.

## Use it from another repository

Pin the action by full commit SHA, not by a tag or branch. Take the 40-character SHA from the commit page on GitHub,
or from `rev-parse HEAD` in a checkout of the commit you want:

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
  stops with an error if they differ. The checksum guards against a corrupted download or a single swapped asset. It
  does not protect against a compromised release, because `SHA256SUMS` comes from the same place as the binary. Other
  runners get an error that points at the options below.
- `version`: the release tag to download. The default (`v0.1-RC2`) is the release this action's commit was written
  against. Pin the action commit and the version together. The tag is checked to be a plain `v...` name before it is
  used in a URL, and downloads are HTTPS only.
- `from-source: true`: build the CLI with `cargo` from the action's own checkout instead. Needs a Rust toolchain on
  the runner and takes longer.
- `hairspring-bin`: a path to a binary you installed in an earlier step. When set, nothing is downloaded or built.

## One run, one report

`hairspring test` takes a single `--format`, so the action makes one run and writes one report. Set `format` to
`sarif` (the default; the only format that is uploaded as code-scanning alerts), `junit` or `text`. The report is
written to `hairspring.sarif`, `hairspring-junit.xml` or `hairspring.txt` and attached to the `hairspring-results`
artifact. The action cannot give you both from one call, and calling it twice in one job does not work: it always uploads an
artifact named `hairspring-results`, a second upload under that name is refused by `actions/upload-artifact` v4, and
the first call already fails the job on a violation so a second step would not run. If you want both SARIF and
JUnit, use the workflow in [docs/ci.md](../../docs/ci.md#the-same-job-without-the-action), which runs
`hairspring test` once per format with the same `--seed`. (Not run on GitHub.)

The report goes to a file, so the seed is printed to the job log as `hairspring seed: N`. The default `seed` is the
workflow run number, so each run explores a different seed and a rerun of the same run repeats it. Set `seed: ""` for a
random seed (still printed). When `seeds` is set it replaces `seed`, because the CLI does not accept both.

If the job fails, the last step of the action says why: exit 1 means a property was violated; anything else means
hairspring did not finish (or an earlier step failed) and there may be no report.

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
| `seed` | run number | Exploration seed, printed in the log; empty means random |
| `config` | `hairspring.toml` | Path to the configuration |
| `format` | `sarif` | `sarif`, `junit` or `text` |
| `sweep` | `false` | Run a sweep |
| `seeds` | empty | Sweep seed range, `FIRST..END`; replaces `seed` |
| `version` | `v0.1-RC2` | Release tag to download |
| `from-source` | `false` | Build with cargo instead of downloading |
| `hairspring-bin` | empty | Use this binary; skip install |
| `upload-sarif` | `true` | Upload SARIF (only when `format` is `sarif`) |

Output: `exit-code` (0 = all properties held, 1 = a violation was found, 2 = error).

## Notes

- A failing run uploads `.hairspring/bundles/*.hairspring`; download one and run `hairspring replay <file>` locally to
  reproduce exactly.
- The action's own third-party steps (`github/codeql-action/upload-sarif`, `actions/upload-artifact`) are pinned to
  full commit SHAs with the tag in a comment.
- This repository's own CI does not use the action and there is no self-test job. The shell in `action.yml` was
  checked with `shellcheck`, and the install and run steps were run locally (the install step against the real
  release and against a local fixture). `actionlint` checks workflow files only, so it was not used on `action.yml`.
- Maintainers: when cutting a release, bump the default of the `version` input in `action.yml` to the new tag and
  update the `version` row above.
