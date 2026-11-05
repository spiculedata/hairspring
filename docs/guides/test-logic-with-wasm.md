# Test your own logic with wasm, on a Mac

The `wasm` backend is the way to explore your own logic on a Mac, or on any machine, with no Hermit, no Linux and no
hardware counters. You write the part you want tested as a WebAssembly module that asks Hairspring to make every
decision that could differ between runs (which thread goes next, whether a message is lost). Hairspring picks the
answers from a seed, records them, replays them, and shrinks a failing run to a minimal list of decisions.

The backend is marked **Working** in the README's backend table. What it is not is a way to run your real program under
real threads: see [what wasm cannot do](#what-wasm-cannot-do). To run an unmodified Linux program you need the `process`
backend ([is my machine suitable?](is-my-machine-suitable.md)).

Everything on this page was run on an Apple silicon Mac, with the released `v0.1-RC1` binary and with a build of
`main` (`hairspring 0.1.0 (0ba59f0 2026-11-05)`), unless it says otherwise. The compiled guests were built in a
`linux/amd64` Ubuntu 24.04 container and then run on the Mac.

## The guest contract

From `crates/hairspring-wasm/src/lib.rs`. A guest imports from the module `hairspring`:

| Import | Meaning |
|---|---|
| `choose(n: i32) -> i32` | A decision in `0..n` (scheduling, message loss, ...), recorded for replay and shrinking |
| `rand_u64() -> i64` | Seeded randomness |
| `now_ms() -> i64` | A virtual clock |
| `sleep_ms(ms: i64)` | Advance the virtual clock |
| `log(ptr: i32, len: i32)` | A log line: `len` bytes of UTF-8 at `ptr` in the guest's memory |
| `fail(ptr: i32, len: i32)` | A violated property, with the message at `ptr`; stops the run |

It exports `run() -> i32` and a `memory`. `run` returning 0 is success. Beyond that, from the backend's source:

- a non-zero return from `run` is a failure (`exit code N`);
- a call to `fail` is a failure that carries your message, so it is the way to say what went wrong;
- a trap is a failure;
- a run that executes more than the step budget of instructions is a failure that says `possible infinite loop`;
- a missing `run` export is reported as a trap.

Import only what you use. The `choose`, `rand_u64`, `now_ms`, `sleep_ms`, `log` and `fail` functions are the only
things the guest can ask the host for; everything else in WebAssembly is deterministic, and it has no threads. Your
"threads" are therefore sequences of steps that your own code interleaves, with `choose` deciding the interleaving.

## An example guest: a lost-update counter

Two "threads" each read a counter and then write back the value plus one. Between the read and the write, either can be
the one that moves next. The property is that the counter ends at 2. `choose(2)` decides which thread takes each
half-step, so the module explores the interleavings, and one of them loses an update.

This is `guest.wat`. Hairspring reads `.wat` text directly, so there is no tool to install:

```wat
(module
  (import "hairspring" "choose" (func $choose (param i32) (result i32)))
  (import "hairspring" "fail" (func $fail (param i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "lost update")
  ;; Two "threads", each doing: read the counter, then write read+1.
  ;; pa and pb are each thread's progress: 0 = not started, 1 = has read, 2 = has written.
  ;; Every time both threads can still move, choose(2) decides which one takes the next half-step.
  (func (export "run") (result i32)
    (local $counter i32) (local $a i32) (local $b i32) (local $pa i32) (local $pb i32)
    (block $done
      (loop $step
        (br_if $done (i32.and (i32.eq (local.get $pa) (i32.const 2)) (i32.eq (local.get $pb) (i32.const 2))))
        (if (i32.and (i32.lt_u (local.get $pa) (i32.const 2))
                     (i32.or (i32.eq (local.get $pb) (i32.const 2)) (i32.eqz (call $choose (i32.const 2)))))
          (then
            (if (i32.eqz (local.get $pa))
              (then (local.set $a (local.get $counter)))
              (else (local.set $counter (i32.add (local.get $a) (i32.const 1)))))
            (local.set $pa (i32.add (local.get $pa) (i32.const 1))))
          (else
            (if (i32.eqz (local.get $pb))
              (then (local.set $b (local.get $counter)))
              (else (local.set $counter (i32.add (local.get $b) (i32.const 1)))))
            (local.set $pb (i32.add (local.get $pb) (i32.const 1)))))
        (br $step)))
    ;; The property: no update was lost. If it does not hold, report it (this stops the run).
    (if (i32.ne (local.get $counter) (i32.const 2))
      (then (call $fail (i32.const 0) (i32.const 11))))
    (i32.const 0)))
```

The project's `hairspring.toml` has one target. The key for the module is `module`; the path is read relative to where
you run the command:

```toml
[target.counter]
kind = "wasm"
module = "guest.wat"
```

Run it. Output from the `main` build, run here:

```text
$ hairspring test counter --seed 1
hairspring: exploring counter (seed 1)
FAILED  counter (wasm)
  lost update; shrunk from 4 to 1 choices
  steps (5):
      1. choose(2) -> 1
      2. choose(2) -> 0
      3. choose(2) -> 0
      4. choose(2) -> 0
      5. result: lost update
  bundle:
    hairspring-bundle v2
    seed=1
    build=0.1.0
    target=counter
    backend=wasm
    param.failure_key=violation:lost update
    param.fuel=200000
    param.module_hash=8c0847418e2fb3c5
    path=0000000000000001
  bundle written: .hairspring/bundles/counter-2727de2a.hairspring  (replay with `hairspring replay .hairspring/bundles/counter-2727de2a.hairspring`)
$ echo $?
1
```

The released `v0.1-RC1` binary printed the same lines, with the same bundle name. The run found the failure on the
first try and shrank four recorded decisions to one: the later decisions were not needed to reproduce it. Exit status 1
means a property was violated (0 passed, 2 is an error).

Replay it. The same bundle, on both builds:

```text
$ hairspring replay .hairspring/bundles/counter-2727de2a.hairspring
replaying 5 steps on target `counter` (backend wasm)
    1. choose(2) -> 1
    2. choose(2) -> 0
    3. choose(2) -> 0
    4. choose(2) -> 0
    5. result: lost update
reproduced: the property is still violated
$ echo $?
1
```

Ask whether the target is deterministic: the same seed must give the same run every time. The output differs in one
phrase between the two builds, run here:

```text
$ hairspring doctor counter            # main
counter: deterministic over 8 seeds
$ hairspring doctor counter            # v0.1-RC1
counter: deterministic over 8 seeds x 0 steps
```

The `x 0 steps` in the released binary is wording that was dropped on `main`; the meaning is the same. Exit status 0.

Other results from the same build of `main`, for a one-instruction module that returns 3 and for a module that loops
for ever:

```text
$ hairspring test three
FAILED  three (wasm)
  exit code 3; shrunk from 0 to 0 choices
$ hairspring test spin
FAILED  spin (wasm)
  did not finish within 200000 instructions (possible infinite loop); shrunk from 0 to 0 choices
$ hairspring test missing                # the module path does not exist
ERROR   missing: cannot read module `nope.wat`: No such file or directory (os error 2)
```

The last one exits 2; the first two exit 1. The 200,000 is the default step budget, which the bundle records as
`param.fuel`.

## Compile your own logic to wasm

A `.wat` file is fine for a small model. For anything larger, write the logic in a language that compiles to
WebAssembly and point `module` at the `.wasm` file. Both outlines below were run: the guest is the same lost-update
counter, built in an Ubuntu 24.04 `linux/amd64` container and copied to the Mac, where `hairspring test` found the same
failure. The compilers are not needed on the machine that runs `hairspring`.

### From C

The imports are declared with the `import_module` and `import_name` attributes, and `run` is exported by name. This is
`guest.c`:

```c
#define IMPORT(name) __attribute__((import_module("hairspring"), import_name(name)))

IMPORT("choose") int choose(int n);
IMPORT("fail") void fail(const char *message, int length);

static const char LOST[] = "lost update";

/* Two "threads", each doing: read the counter, then write read+1. phase is each thread's progress
   (0 = not started, 1 = has read, 2 = has written); choose(2) picks who takes the next half-step. */
__attribute__((export_name("run"))) int run(void) {
    int counter = 0, seen[2] = {0, 0}, phase[2] = {0, 0};
    while (phase[0] < 2 || phase[1] < 2) {
        int t = phase[0] == 2 ? 1 : phase[1] == 2 ? 0 : choose(2);
        if (phase[t] == 0) seen[t] = counter; else counter = seen[t] + 1;
        phase[t]++;
    }
    if (counter != 2) fail(LOST, sizeof LOST - 1);
    return 0;
}
```

Build it with clang and wasm-ld (the container had Ubuntu's `clang` 18.1.3 and `lld`, installed with
`apt-get install -y clang lld`):

```bash
clang --target=wasm32 -O2 -nostdlib -Wl,--no-entry -Wl,--allow-undefined -o guest-c.wasm guest.c
```

That gave a 529-byte module. `-Wl,--allow-undefined` leaves the `hairspring` imports for the host to provide; `wasm-ld`
exports the memory as `memory` by default. On the Mac, with the `main` build:

```text
$ hairspring test counter-c --seed 1
FAILED  counter-c (wasm)
  lost update; shrunk from 3 to 1 choices
```

and `hairspring doctor counter-c` printed `counter-c: deterministic over 8 seeds`. The Mac's Apple clang knows the
`wasm32` target but has no `wasm-ld` on its path, so the link step was not run there.

### From Rust

Use the `wasm32-unknown-unknown` target and a `cdylib`. `Cargo.toml`:

```toml
[package]
name = "guest"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[profile.release]
panic = "abort"
opt-level = "s"
```

`src/lib.rs`:

```rust
#[link(wasm_import_module = "hairspring")]
extern "C" {
    fn choose(n: i32) -> i32;
    fn fail(message: *const u8, length: i32);
}

const LOST: &str = "lost update";

// Two "threads", each doing: read the counter, then write read+1. phase is each thread's progress
// (0 = not started, 1 = has read, 2 = has written); choose(2) picks who takes the next half-step.
#[no_mangle]
pub extern "C" fn run() -> i32 {
    let (mut counter, mut seen, mut phase) = (0, [0, 0], [0, 0]);
    while phase[0] < 2 || phase[1] < 2 {
        let t = if phase[0] == 2 { 1 } else if phase[1] == 2 { 0 } else { unsafe { choose(2) as usize } };
        if phase[t] == 0 { seen[t] = counter } else { counter = seen[t] + 1 }
        phase[t] += 1;
    }
    if counter != 2 {
        unsafe { fail(LOST.as_ptr(), LOST.len() as i32) }
    }
    0
}
```

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown       # target/wasm32-unknown-unknown/release/guest.wasm
```

That gave a 22 KB module (run in the container, with Rust 1.99.1 from rustup; Homebrew's Rust on the Mac has no rustup,
so it has no wasm target). On the Mac, with the `main` build, `hairspring test counter-rs --seed 1` printed
`FAILED  counter-rs (wasm)` and `lost update; shrunk from 3 to 1 choices`, and `doctor` printed
`counter-rs: deterministic over 8 seeds`. The released binary found the same failure in a fresh directory.

### Other languages

Any language that compiles to WebAssembly and can import functions by module and name can write a guest. Others were not
tried here. A guest needs to export `run` and `memory` and to import only the functions above.

## How often, not just whether

A single `test` stops at the first failing seed. A sweep runs a range of seeds and counts them. Run here, with the
`main` build:

```text
$ hairspring test counter --sweep --seeds 1..21
hairspring: sweeping counter (seeds 1..21)
FAILED  counter (wasm)
  lost update; shrunk from 4 to 1 choices
  ...
  sweep: 20 trials: 9 pass, 11 violation, 0 inconclusive, 0 unusable, 0 infra
```

That is 11 of the 20 seeds from 1 up to but not including 21. The seeds that passed are runs where the interleaving did
not lose an update. A pass says only that the seeds tried did not fail. How a sweep is read, and `confirm`, are in
[Sweeps and confirmation](https://github.com/spiculedata/hairspring/wiki/Sweeps-and-Confirmation); `confirm` of a wasm
target was not tried here.

## What wasm cannot do

- **It is not your real program.** The guest is a model you write, or logic you compile for the purpose. Real threads,
  real sockets, the operating system's scheduler, libc and the kernel are not in it. A pass says nothing about them.
- **It has no threads.** WebAssembly in this backend runs one flow of control. Concurrency is something your guest
  simulates by interleaving steps with `choose`. If your guest interleaves wrongly, you are testing the wrong model.
- **Only the host-provided sources are controlled.** Time, randomness and decisions must come from the imports. A guest
  that computes its own pseudo-randomness from a fixed seed is deterministic but not explorable.
- **There are limits.** The default step budget is 200,000 instructions, the module may be at most 16 MiB, and the guest's
  memory is capped at 64 MiB (the defaults in the backend's source).
- **It is not the `sim` backend.** `sim` targets are state machines written in Rust against the `Sim` trait and
  registered in your own binary; `wasm` targets are modules loaded from a file.

To run an unmodified program, see the `process` backend: [build a Linux executable](build-a-linux-executable.md),
[testing a service](testing-a-service.md) and [is my machine suitable?](is-my-machine-suitable.md).
