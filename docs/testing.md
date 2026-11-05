# Testing

## Running the tests

```sh
cargo test --workspace
```

Tests that need Docker, PostgreSQL or Hermit are marked in their own files and need a Linux x86-64 machine for the
parts that run guests.

## Generated-input tests for guest output

The code that reads untrusted guest output has seeded, generated-input tests. They need no extra dependency: a small
xorshift generator in `crates/hairspring-props/tests/common/fuzz_rng.rs` builds the inputs. Every failing assertion in
a generated loop is reported as `case N, seed S (HAIRSPRING_FUZZ_SEED=B): ...`, so the input can be rebuilt from the
message.

| File | What it feeds | What it checks |
|---|---|---|
| `crates/hairspring-props/tests/protocol_fuzz.rs` | event lines, whole output streams, digest bytes, catalog updates, run summaries | no panic; every accepted event is inside the limits in `limits.rs`; each candidate line is counted exactly once or more, never lost; refused lines degrade the trial, including a single over-long line in an otherwise clean trial; the 64-process and 256-thread caps hold and every line past them is counted; every kind of event is accepted often enough to be exercised; digests round-trip and only canonical bytes decode; run counts never exceed the trials added |
| `crates/hairspring-props/tests/figures_fuzz.rs` | counts up to `u64::MAX` and awkward floats | Wilson intervals stay finite, ordered and around the observed rate; rounded percentages never look better than the evidence |
| `crates/hairspring-hermit/src/stream_fuzz_tests.rs` | raw stdout and stderr bytes in random read sizes | where reads end changes nothing; stored output and event lines stay inside their caps; the events the splitter keeps fold to what the whole stream folds to |

By default each property runs a few thousand cases and the three files together take a few seconds. Two environment
variables change that:

```sh
# One million cases per property, in a release build (several minutes), with a seed that differs from the default.
HAIRSPRING_FUZZ_CASES=1000000 HAIRSPRING_FUZZ_SEED=12345 \
  cargo test --release -p hairspring-props --test protocol_fuzz --test figures_fuzz
HAIRSPRING_FUZZ_CASES=200000 cargo test --release -p hairspring-hermit --lib stream_fuzz
```

`HAIRSPRING_FUZZ_CASES` is the number of cases for each property (a few properties that enumerate fixed inputs ignore
it). `HAIRSPRING_FUZZ_SEED` is added to every case number, so a new value gives new inputs. When a case fails, the
message names the case, the seed and the value of `HAIRSPRING_FUZZ_SEED` that was in force. Rerun the same test with that
`HAIRSPRING_FUZZ_SEED` and a `HAIRSPRING_FUZZ_CASES` above the case number to get the same input.

The generators are weighted so that a default run reaches every kind of event, the process and thread caps and a lone
refused line. `generated_lines_never_panic_and_accepted_events_obey_every_limit` prints how many events of each kind
were accepted (run it with `-- --nocapture` to see the table) and fails if any kind falls below a floor.

Coverage-guided fuzzing with cargo-fuzz needs a nightly toolchain and is not set up here.

## Mutation testing

Line coverage shows which code ran, not whether a test would notice if that code were wrong.
[cargo-mutants](https://mutants.rs) changes the code one small step at a time (swapping `<` for `<=`, replacing a
function body with a constant) and runs the tests against each change. A change that no test fails on is a *missed*
mutant, and points at behaviour with no test pinning it.

cargo-mutants is a developer tool, not a dependency of this repository:

```sh
cargo install cargo-mutants --locked
```

Run it on one crate, one or two files at a time, with a bounded number of jobs and a timeout, and choose the tests
that cover those files. Each job builds its own copy of the tree, so a whole workspace takes a long time.

```sh
# Pure logic in the property crate: skip the SDK tests, which compile guest programs and are slow.
cargo mutants -p hairspring-props --jobs 4 --timeout 60 \
  -f '**/hairspring-props/src/percent.rs' -f '**/hairspring-props/src/tally.rs' \
  -- --test percent --test fold --test mutation_gaps

# A crate whose logic is covered by unit tests.
cargo mutants -p hairspring-server --jobs 4 --timeout 120 \
  -f '**/hairspring-server/src/ratelimit.rs' -- --lib
```

Use `cargo mutants -p <crate> --list` to see how many mutants a selection has before starting it. Results are written
to `mutants.out/` in the current directory, which is not git-ignored: pass `--output <directory outside the repository>`
to keep them out of a commit. `missed.txt` lists the mutants to look at. Do not run it against tests that
need Docker, PostgreSQL or Hermit.

For each missed mutant, decide which of two cases it is:

- A real gap: the changed code behaves differently in a way a user, a caller or a security rule would care about.
  Write a test that fails on the mutant and passes on the real code. To check, apply the change by hand (or run
  `cargo mutants -p <crate> -F '<text of the mutant>'`) and see the test fail.
- An equivalent mutant: the change cannot alter any observable result (for example, a field set to a value that a later
  line overwrites). Leave the code alone and do not write a test for it.

A mutant that times out counts as caught: the change made the code loop or stall.
