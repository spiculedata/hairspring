# Testing

## Running the tests

```sh
cargo test --workspace
```

Tests that need Docker, PostgreSQL or Hermit are marked in their own files and need a Linux x86-64 machine for the
parts that run guests.

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
