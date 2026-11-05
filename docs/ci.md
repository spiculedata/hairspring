# Hairspring in CI

How to run `hairspring test` on every push or pull request, get the findings back as annotations or a test report, keep
the replayable bundles, and reproduce a CI failure on your own machine. Every command here is from `hairspring test
--help` or was run while writing this page; the last section says what was not run.

Which backend decides where the job can run:

| Target kind | Where it can run in CI | Needs |
|---|---|---|
| `sim` (built-in demos, or a state machine you wrote) | A GitHub-hosted runner, or any Linux x86-64 or macOS arm64 machine | The `hairspring` release binary |
| `wasm` | The same | The binary and your `.wasm` or `.wat` file |
| `process` | A bare-metal Linux x86-64 machine you control, with Hermit and performance counters | See [Process targets](#process-targets-on-a-self-hosted-runner) |

Exit codes of `hairspring test`, which every CI system uses to pass or fail the job:

| Exit | Meaning |
|---|---|
| 0 | every property held on the seeds that were run |
| 1 | a property was violated |
| 2 | an error: an unknown target, a bad flag or configuration, a missing Hermit |

A pass says only that the seeds you ran did not fail; it is not a proof that the bug is absent. Ctrl-C (or a CI
system cancelling the job) stops the run and exits 130.

## A GitHub Actions workflow for simulation and wasm targets

This is the whole file. Save it as `.github/workflows/hairspring.yml` in your repository, next to a `hairspring.toml`
(or use only the built-in targets). Replace `<full commit sha>` with the commit of this repository you have chosen to
use (see below).

```yaml
name: hairspring

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  hairspring:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write   # lets the SARIF upload create code-scanning alerts
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: lock-lease pubsub-atomic
          # seed defaults to the workflow run number: a rerun of one run repeats it, the next run explores another.
```

What it does, in order: downloads the release binary for the runner, checks its sha256 against the release's
`SHA256SUMS` and stops if they differ, runs `hairspring test` once, uploads the findings to code scanning (SARIF),
uploads the replayable bundles and the report as an artifact named `hairspring-results`, and fails the job when a
property was violated. The seed is printed in the job log as `hairspring seed: N`. The action's reference (every
input, the exit-code output, what each step does) is in
[integrations/github-action/README.md](../integrations/github-action/README.md).

**Pin the action by full commit SHA**, not by a tag or a branch. A tag or branch can be moved to different code after
you reviewed it; a 40-character commit SHA cannot. Take the SHA from the commit page on GitHub, or from
`git rev-parse HEAD` in a checkout of the commit you want. The action downloads the release named by its `version`
input (the default is the release it was written against), so pin the action commit and, if you set it, `version`
together. The checksum guards against a corrupted download or one swapped asset. It does not protect against a
compromised release, because `SHA256SUMS` comes from the same place as the binary.

The findings appear under the repository's Security tab, Code scanning. Each alert is attached to `hairspring.toml`
(that is the location the SARIF file names), with the target, the property message and the shrunk step count. The
repository needs code scanning to be available to it: GitHub offers it free for public repositories, and for private
ones it depends on your plan and settings (see GitHub's documentation). A workflow started by a pull request from a
fork gets a read-only token, so the SARIF upload step cannot write alerts there; the job still fails on a violation
and the bundles are still attached. None of the GitHub-side behaviour in this paragraph was run here.

### JUnit instead of SARIF

Set `format: junit` to get a JUnit XML report instead. Only SARIF can be uploaded as code-scanning alerts, so the
upload step is skipped; the report is `hairspring-junit.xml` in the `hairspring-results` artifact.

```yaml
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: lock-lease pubsub-atomic
          format: junit
```

The report is one `testsuite` named `hairspring` with one `testcase` per target. A failing target has a `failure`
element whose message is the property message and whose body lists the shrunk steps and the bundle text. A passing
run looks like this (real output of `hairspring test pubsub-atomic --format junit`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="hairspring" tests="1" failures="0" errors="0">
  <testcase classname="hairspring" name="pubsub-atomic" time="1.116">
  </testcase>
</testsuite>
```

and a failing one starts like this (`hairspring test pubsub-buggy --seed 5 --format junit`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="hairspring" tests="1" failures="1" errors="0">
  <testcase classname="hairspring" name="pubsub-buggy" time="0.028">
    <failure message="invariant violated; shrunk from 23 to 8 steps">1. Deliver(1, 0)
2. Tick
...
```

To show the report on the pull request, use a JUnit report publisher from the Actions marketplace, for example
[`mikepenz/action-junit-report`](https://github.com/mikepenz/action-junit-report) or
[`dorny/test-reporter`](https://github.com/dorny/test-reporter) (pin whichever you choose by commit SHA, as above).
Neither was tried here; read their documentation for how to point them at `hairspring-junit.xml`.

`hairspring test` takes one `--format` per run, so a single run gives one report. If you want both SARIF and JUnit
from the same exploration, use the workflow in the next section and run the command twice with the same `--seed`; a
sim run with a fixed seed repeats exactly, and each run costs its own time.

### The same job without the action

If you would rather not depend on this repository's action, these are the same steps spelled out. The shell parts
were run locally; see the last section.

```yaml
name: hairspring

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  hairspring:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write
    env:
      HAIRSPRING_VERSION: v0.1-RC1
      HAIRSPRING_ASSET: hairspring-linux-x86_64
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4

      - name: Install hairspring
        run: |
          set -euo pipefail
          base="https://github.com/spiculedata/hairspring/releases/download/${HAIRSPRING_VERSION}"
          cd "$RUNNER_TEMP"
          curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
            --remote-name "$base/$HAIRSPRING_ASSET" --remote-name "$base/SHA256SUMS"
          grep "$HAIRSPRING_ASSET" SHA256SUMS | sha256sum -c
          mkdir -p "$RUNNER_TEMP/bin"
          install -m 0755 "$HAIRSPRING_ASSET" "$RUNNER_TEMP/bin/hairspring"
          echo "$RUNNER_TEMP/bin" >> "$GITHUB_PATH"

      - name: Run hairspring
        env:
          SEED: ${{ github.run_number }}
        run: |
          echo "hairspring seed: $SEED"
          set +e
          hairspring test lock-lease pubsub-atomic --seed "$SEED" --format sarif > hairspring.sarif
          code=$?
          [ -s hairspring.sarif ] || rm -f hairspring.sarif
          exit "$code"

      - name: Upload findings to code scanning
        if: ${{ always() && hashFiles('hairspring.sarif') != '' }}
        uses: github/codeql-action/upload-sarif@9418214df1f2c4b8128c770156c409013b07d6b9 # v3
        with:
          sarif_file: hairspring.sarif
          category: hairspring

      - name: Upload bundles and report
        if: ${{ always() }}
        uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: hairspring-results
          path: |
            .hairspring/bundles/
            hairspring.sarif
          if-no-files-found: ignore
```

The job fails because the `Run hairspring` step exits with the command's own code (1 for a violation, 2 for an error),
and the two upload steps use `always()` so they still run. For JUnit, change `--format sarif > hairspring.sarif` to
`--format junit > hairspring-junit.xml` and add that file to `path`; drop the code-scanning step. The upload actions
are pinned to the same commits the action uses.

## A regression sweep

`hairspring test` normally stops at the first failure. A sweep runs a whole range of seeds past every failure, stores
every trial, and reports how many seeds failed. Use it to check that a fixed bug stays fixed across many schedules, or
to put a rate on a flaky property. The flags (from `hairspring test --help`):

| Flag | Meaning |
|---|---|
| `--sweep` | run the whole trial budget past every failure |
| `--seeds A..B` | the seeds to run: `A` up to but not including `B`, so `100..200` is 100 trials. Cannot be combined with `--seed` |
| `--trials N` | how many trials a sweep runs when you give `--seed` instead of `--seeds` (default: the target's own budget, at most 10000) |
| `--seed N` | with `--sweep` and no `--seeds`, a sweep runs consecutive seeds from `N` |
| `--format` | `text`, `json`, `junit` or `sarif` |
| `--bundle-dir DIR` | where bundles are written (default `.hairspring/bundles`) |

With the action:

```yaml
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: lock-lease
          sweep: "true"
          seeds: 100..200
```

or on any machine: `hairspring test lock-lease --sweep --seeds 100..200`. The exit code is the same three values as for
a single run, so a sweep fails the job exactly when at least one seed violated a property:

```text
$ hairspring test lock-lease --sweep --seeds 100..200
hairspring: sweeping lock-lease (seeds 100..200)
FAILED  lock-lease (sim)
  invariant violated; shrunk from 22 to 8 steps
  ...
  sweep: 100 trials: 0 pass, 100 violation, 0 inconclusive, 0 unusable, 0 infra
  failing seeds: 100, 101, 102, 103, 104, 105, 106, 107, 108, 109 (first 10 of 100 violations)
  hits: 100/100 rated trials (100.00%), Wilson 95% interval [96.30%, 100.00%]
  bundle written: .hairspring/bundles/lock-lease-ee911c58.hairspring  (replay with `hairspring replay .hairspring/bundles/lock-lease-ee911c58.hairspring`)
exit 1
```

(`lock-lease` is a built-in demo with a planted bug, so every seed fails; on your own target a fixed build prints
`PASSED` and exits 0.) A sweep writes one bundle for the lowest failing seed, and one more for each later seed at which
a different property failed for the first time. A passing sweep says only that those seeds did not fail, and the line
that says how many trials ran can be lower than you asked for when a step or wall-clock limit ended the sweep early.

## Process targets on a self-hosted runner

A `kind = "process"` target runs an unmodified Linux program under Hermit, which needs Linux on x86-64 with hardware
performance counters. Use a self-hosted runner on a bare-metal Linux x86-64 machine that you have checked.
**GitHub-hosted runners were not tested for process targets**, and ordinary virtual machines and containers often do
not expose the counters, so do not assume a hosted runner can run them.

On the machine, once, as the user the runner runs as:

1. Install the runner software as GitHub describes in
   [Adding self-hosted runners](https://docs.github.com/en/actions/hosting-your-own-runners/managing-self-hosted-runners/adding-self-hosted-runners).
2. Check the machine is suitable, then build and install Hermit, from a checkout of this repository
   (the steps and what the installer verifies are in the [quickstart](quickstart.md#install)):

   ```bash
   bash scripts/install-hermit.sh check
   bash scripts/install-hermit.sh install --install-toolchain
   ```

3. Tell Hairspring which Hermit to use, and check the whole picture. The installer prints the path; its default is
   `~/.local/share/hairspring/hermit/bin/hermit` for the installing user.

   ```bash
   export HAIRSPRING_HERMIT="$HOME/.local/share/hairspring/hermit/bin/hermit"
   hairspring doctor --process-backend
   ```

4. Put the program you test, built, at the absolute path `hairspring.toml` names, the same path on every run. A
   recorded schedule depends on the host (CPU, kernel, loader, Hermit build), so keep the machine's packages stable
   between runs.

The workflow targets the runner by its labels. Give the job the Hermit path with `env` (use the absolute path from
step 3; `env:` does not expand `$HOME`):

```yaml
name: hairspring-process

on:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  hairspring-process:
    runs-on: [self-hosted, linux, x64]
    permissions:
      contents: read
      security-events: write
    env:
      HAIRSPRING_HERMIT: /home/ci/.local/share/hairspring/hermit/bin/hermit
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4
      # Build the program the target runs here, at the path hairspring.toml names.
      - uses: spiculedata/hairspring/integrations/github-action@<full commit sha>
        with:
          targets: my-process-target
          config: hairspring.toml
```

Notes for this setup:

- **Bundles are kept** as the `hairspring-results` artifact, which the action uploads even when the job fails. The
  runner's workspace is not a place to keep them: it is reused and cleaned by the runner.
- **Time limits.** A target's budget has `wall_secs`, 60 by default and at most 600, for the whole run. The default
  allows only a few Hermit trials; set `wall_secs` under `[target.NAME.budget]` or pass `--budget-secs`. A trial that
  does not finish within `trial_timeout_secs` is inconclusive, never a pass.
- **A sweep fits here.** Run the same fixed build over a range, for example `sweep: "true"` with `seeds: 1..101`, and
  read the rate in the log. Each trial under Hermit is slower than native.
- **Do not run untrusted code on this machine.** A self-hosted runner runs whatever a workflow tells it to; GitHub's
  guidance is to avoid self-hosted runners for public repositories, because a pull request from a fork can run
  arbitrary code on them. A process target also runs your program with the runner user's privileges. The recorded
  measurements ran Hermit as root and running it unprivileged was not tested; see the [quickstart](quickstart.md#what-you-need).
- If the job cannot start Hermit it says so and exits 2: see the Troubleshooting table in the
  [quickstart](quickstart.md#troubleshooting).

## Not using GitHub Actions

Any CI system can run the CLI. All it needs is the binary, a check of its checksum, and the exit codes above. This is
a plain shell script that installs a release on Linux x86-64 and runs a target; run it as one job step, or as the
body of a job in GitLab CI, Jenkins, CircleCI or anything else that can run a shell.

```sh
set -eu
version=v0.1-RC1
asset=hairspring-linux-x86_64
base="https://github.com/spiculedata/hairspring/releases/download/$version"

curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
  --remote-name "$base/$asset" --remote-name "$base/SHA256SUMS"

# Check the one binary you downloaded. (`sha256sum -c SHA256SUMS` alone fails, because it also looks for the
# macOS binary you did not download.) On macOS use `shasum -a 256 -c` in place of `sha256sum -c`.
grep "$asset" SHA256SUMS | sha256sum -c

chmod +x "$asset"
mv "$asset" hairspring

# Run. The report goes to stdout, so keep it out of the log with a redirect; the bundle note goes to stderr.
set +e
./hairspring test lock-lease pubsub-atomic --seed "${CI_PIPELINE_ID:-1}" --format junit > hairspring-junit.xml
code=$?
set -e
echo "hairspring exit code: $code"
exit "$code"
```

Notes:

- Use a seed your CI system can supply (a pipeline or build number), or a fixed one. With `--format junit` or `sarif`
  the command writes only the report to stdout and prints nothing about the seed, so echo it into the log yourself.
- Exit 1 fails the job without any extra work, and 2 does too; keep the report and `.hairspring/bundles/` as job
  artifacts **even when the job fails** (in GitLab CI, `artifacts: when: always`; in Jenkins, archive them in a `post
  { always { ... } }` block).
- **JUnit** is read natively by most CI systems: GitLab CI takes it with `artifacts: reports: junit:
  hairspring-junit.xml`, Jenkins with the JUnit plugin's `junit 'hairspring-junit.xml'` step. Check their
  documentation for the exact keys; neither was run here.
- **SARIF** is only useful where something reads it: GitHub code scanning does, and so do several other viewers and
  IDE plug-ins. Otherwise keep it as an artifact, or use `--format text` for the log and `--format json` for scripts.
- **The checksum** protects against a corrupted download only. It comes from the same release as the binary; to pin
  a hash you obtained some other way, give it to `scripts/install.sh --version TAG --sha256 HEX` or compare it
  yourself.
- The hosted-runner limits above apply to any CI service: a `process` target needs the bare-metal machine described in
  the previous section, whatever runs the job on it.

## Reproducing a CI failure locally

A failing job prints the seed and writes a bundle: a small text file that holds the target, the seed and the recorded
choices. Download `hairspring-results` from the workflow run (or your CI's artifacts), then:

```bash
hairspring replay .hairspring/bundles/pubsub-buggy-3893b41f.hairspring
```

which prints the recorded steps and ends:

```text
reproduced: the property is still violated
```

`replay` exits 1 when the property is still violated, 0 when it is not (on the environment it was recorded in), 2 on an
error, and 3 when it did not reproduce on a changed environment, which is inconclusive and not evidence the bug is
fixed. It reads `hairspring.toml` for targets you defined (`--config PATH`), so run it from a checkout of the same
commit CI ran. For a process target the replay also needs the program at the same path and the same kind of machine;
see [Recorded environment](determinism.md#recorded-environment).

Without the bundle you can still repeat the failing exploration: `hairspring test <target> --seed N` with the seed
from the log (or `--sweep --seeds A..B` for a sweep) runs the same seeds again. For a sim target that gives the same
result; for a process target the answer depends on the machine.

## What to commit

- **Commit** `hairspring.toml` and the workflow file. The seed range of a regression sweep lives in the workflow,
  which is what pins it.
- **Do not commit** `.hairspring/`: it holds the local results database and the bundles of every run. Add it to your
  `.gitignore`.
- **Keep a bundle** only for a bug you mean to keep as a regression case, and attach it to the issue or pull request
  where the bug is discussed rather than to the repository history. A bundle replays exactly only against the program
  it was recorded from. A process bundle holds hashes of the program, its command line and its working directory, which
  can be guessed offline from short paths or arguments; read the table in
  [Recorded environment](determinism.md#recorded-environment) before publishing one.
- Prefer a fixed seed or seed range in the workflow when you want the job to give the same answer on every commit, and
  the run number (the action's default) when you want each run to explore something new. A violation found on one
  run's seed may not appear on the next run's, so do not rerun a red job expecting it to go green: replay its bundle.

## What was run here, and what was not

Run while writing this page, on macOS arm64:

- `hairspring test ... --format junit` and `--format sarif` on built-in simulation targets, a sweep with
  `--seeds`, an unknown target (exit 2), `--seed` with `--seeds` (refused, exit 2), and `hairspring replay` of a
  bundle (exit 1). The outputs quoted above are the real ones.
- The action's install and run steps, taken out of `action.yml` and run in a shell: the release download with the
  checksum check for both the Linux and macOS assets, a refused version string, a refused runner type, and the run
  step with SARIF, JUnit, a sweep, a passing target, an unknown target and a random seed.
- The checksum commands, in the single-file form.
- The three complete workflow files above were checked with `actionlint` (placeholders replaced), and the snippets
  that call the action were checked the same way against this repository's `action.yml` as a local action, which
  validates the input names.
- The shell of the install and run steps of the workflow without the action, and the plain shell script, were run
  against the real v0.1-RC1 release with the macOS asset in place of the Linux one (a Mac was what was to hand), with a
  `sha256sum` that calls `shasum -a 256` standing in for GNU's.

Not run here:

- Any workflow on GitHub: the `uses:` steps, the code-scanning upload, the artifact upload, a fork pull request, and
  the third-party JUnit publishers. The binary download and checksum step is the part most worth watching on a first
  run.
- GitHub-hosted runners (for any target kind) and anything on a self-hosted runner.
- Anything needing Linux x86-64, Hermit and performance counters: process targets, the Hermit installer on a runner,
  and `replay` of a process bundle.
- The GitLab CI, Jenkins and other systems named in the section above.
