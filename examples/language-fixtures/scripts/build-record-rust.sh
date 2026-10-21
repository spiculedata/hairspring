#!/bin/sh
# Generates the BUILD-RECORD of the Rust fixture artifact at-rust: checks B1 to B7 of the fixture contract, in one Markdown file.
#
#   build-record-rust.sh [out-file]
#
# Run it on Linux with the pinned Rust toolchain and the musl target installed (the manual CI job does). It builds at-rust
# twice (two target directories), disassembles the busy loop and every function that calls it, runs the native self-check
# 50 times per mode, runs strace -ff on one buggy run, runs the SDK's tests and conformance, and prints the source of the
# modes. It then captures the evidence for the rest of the alignment checklist from the built artifact, and ticks each
# checklist line only when the evidence that proves it passed (record-lib.sh). Nothing here runs Hermit or touches the
# box. Exits 1 if any step failed.
#
# This script covers at-rust only, and the record says so. The std-only probe program probe-rust has its own record,
# from build-record-probe-rust.sh.
#
# Environment: TARGET is the Rust target (default x86_64-unknown-linux-musl, the pinned static build), RUST_PINNED the
# toolchain version (default 1.99.1), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50),
# ARTIFACT_DIR receives a copy of the pinned binary and its SHA256SUMS. The producer fields come from the GitHub Actions
# environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_*, evidence_*
# and barrier_sources variables are the interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
runs=${RUNS:-50}
target=${TARGET:-x86_64-unknown-linux-musl}
rust_pinned=${RUST_PINNED:-1.99.1}
out=${1:-$root/build/record-rust.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

fixture=$repo/sdks/rust/fixtures/lang_at
artifact=at-rust
lang=rust
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
target_one=$work/target-one
target_two=$work/target-two
binary=$target_one/$target/release/$artifact
# Paths of the checkout are mapped to /src, so the artifact does not depend on where the tree is built.
rustflags="--remap-path-prefix=$repo=/src"

cargo_build() { # cargo_build <target-dir>
  RUSTFLAGS=$rustflags CARGO_TARGET_DIR=$1 cargo build --release --locked --target "$target" -p hairspring-sdk --bin "$artifact" --manifest-path "$repo/Cargo.toml"
}

versions() {
  uname -srm
  got=$(rustc --version) || return 1
  echo "$got"
  rustc -vV | grep -E '^(host|LLVM version):'
  cargo --version
  echo "target: $target"
  case "$got" in
    "rustc $rust_pinned "*) echo "ok   the Rust toolchain is the pinned $rust_pinned" ;;
    *) echo "FAIL the Rust toolchain is not the pinned $rust_pinned"; return 1 ;;
  esac
  objdump --version | head -1
  command -v strace >/dev/null 2>&1 && strace -V 2>&1 | head -1
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
  echo "build command: RUSTFLAGS='$rustflags' CARGO_TARGET_DIR=<dir> cargo build --release --locked --target $target -p hairspring-sdk --bin $artifact"
  echo "--- dependencies of hairspring-sdk (no dependencies: only the crate itself may be listed)"
  deps=$(cargo tree --locked -p hairspring-sdk --target "$target" --manifest-path "$repo/Cargo.toml") || return 1
  echo "$deps"
  [ "$(printf '%s\n' "$deps" | wc -l | tr -d ' ')" -eq 1 ] || { echo "FAIL hairspring-sdk has dependencies"; return 1; }
  echo "ok   hairspring-sdk has no dependencies"
}

build_twice() {
  echo "The checkout path is mapped to /src (--remap-path-prefix), so the sha256 does not depend on the directory the"
  echo "tree is built in; the second build uses a different target directory."
  rm -rf "$target_one" "$target_two"
  cargo_build "$target_one" || return 1
  cargo_build "$target_two" || return 1
  first=$(hash_file "$binary")
  second=$(hash_file "$target_two/$target/release/$artifact")
  command -v file >/dev/null 2>&1 && file "$binary"
  equal_hashes "$artifact" "$first" "$second"
}

# The mangled name of the busy loop ends in 4spin: _ZN...4spin17h<hash>E (legacy mangling) or _RNv...6window4spin (v0).
# The window may be inlined into more than one function (a locked copy and an unlocked copy of the step, the closure that
# runs a worker), so the check reads every function that calls the busy loop.
disassembly() {
  objdump -d --no-show-raw-insn "$binary" >"$work/dis.s" || return 1
  extract '<(_ZN|_RNv)[^>]*4spin[^>]*>:' "$work/dis.s" >"$work/spin.s"
  awk 'BEGIN { RS = ""; ORS = "\n\n" } /call[^\n]*4spin/' "$work/dis.s" >"$work/worker.s"
  [ -s "$work/spin.s" ] && [ -s "$work/worker.s" ] || { echo "the busy loop or a function that calls it was not found"; return 1; }
  cat "$work/spin.s"
  echo "--- every function that calls the busy loop, around each call"
  grep -n -B12 -A12 'call.*4spin' "$work/worker.s"
  sh "$here/check-disasm.sh" loop "$work/spin.s" || { echo "no backward branch in the busy loop"; return 1; }
  echo "ok   the busy loop has a backward branch"
  # Every call to the busy loop must sit between a load and a store of the same address with nothing else between. The
  # mutex is a futex word taken inline (a lock cmpxchg), and there must be a copy of the window with no lock taken in the
  # lines before its load: the buggy path.
  UNLOCKED_PATH=1 sh "$here/check-disasm.sh" window "$work/worker.s" || { echo "a call to the busy loop is not a load -> call -> store of one address, or there is no copy of the window outside the lock"; return 1; }
  echo "ok   every call to the busy loop is load -> call -> store of the same address, with no other call or locked read-modify-write between"
  echo "ok   a copy of the window exists with no lock taken before its load (the buggy path)"
}

native() {
  LANG_AT_REQUIRE_PARALLEL=1 sh "$here/check-native.sh" rust "$binary" "$runs"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace.
events() {
  strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$binary" buggy >"$work/stdout.txt" 2>"$work/stderr.txt" || return 1
  cat "$work"/strace.* >"$work/strace-all.txt"
  lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
  sh "$here/check-writes.sh" "$work/strace-all.txt" >"$work/writes.txt" || { cat "$work/writes.txt"; return 1; }
  cat "$work/writes.txt"
  written=$(cut -d' ' -f1 "$work/writes.txt")
  echo "HAIRSPRING_EVENT lines on stderr: $lines; protocol line writes in the trace: $written"
  [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ]
}

# The SDK's own tests (including the fixture's unit and native tests and the seccomp no-syscall check) in release, then the
# conformance tests of the parser against the SDK's programs. A separate target directory keeps the checkout clean.
goldens() {
  CARGO_TARGET_DIR=$work/target-test cargo test --locked --release -p hairspring-sdk --manifest-path "$repo/Cargo.toml" || return 1
  CARGO_TARGET_DIR=$work/target-test cargo test --locked -p hairspring-props --test rust_sdk --manifest-path "$repo/Cargo.toml"
}

modes() {
  {
    sed -n '/^pub const THREADS/,/^pub const USAGE/p' "$fixture/mode.rs"
    sed -n '/pub fn window/,/^    }/p;/pub fn is_locked/,/^    }/p' "$fixture/mode.rs"
  } | require_lines
}

# The grep of the fixture source for line 11 of the checklist: every file of the fixture directory.
forbidden_reads() {
  sh "$here/check-sources.sh" "$fixture"
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-rust.sh."
artifact_name=$artifact
artifact_sha="<not built>"
evidence_artifact=$binary
barrier_sources=$fixture/window.rs
constant_sources=$fixture/mode.rs

say "Scope: this record covers at-rust. The std-only probe program probe-rust has its own record, produced by"
say "build-record-probe-rust.sh."
say ""
say "Reproducibility: the checkout path is mapped to /src with RUSTFLAGS=--remap-path-prefix=<checkout>=/src. That flag is part"
say "of the build command (B1 prints it in full) and is not in a plain \`cargo build\`: an artifact built without it has another"
say "sha256. The Rust standard library is the toolchain's own. The pinned artifact comes from one named producer: the one"
say "named above."
say ""
step B1 "toolchain, build command, dependencies and two builds with equal sha256" versions
step B1 "the build, twice" build_twice
artifact_sha=$(publish_artifact "$binary" "$artifact") || artifact_sha="<not built>"
step B2 "the busy loop survived and the window's load and store are separate instructions" disassembly
not_applicable B3 "seccomp self-test" "Not applicable to Rust: the runtime makes system calls on other threads. The no-syscall window is a code-review statement: the window is a load, \`spin\` (no call, no allocation) and a store, checked in B2. The SDK's own seccomp test (sdks/rust/tests/no_syscall.rs) runs in B6." yes
say "Exit-3 paths: a closed stdout is exercised by B4 (exit 3 required). A failed thread creation is covered by the fixture's"
say "unit tests (a worker that panics or cannot start makes the run an invariant failure, exit 3)."
say ""
step B4 "$runs native runs of buggy and fixed, wide as a contract check (at least two CPUs required)" native
if command -v strace >/dev/null 2>&1; then
  step B5 "each HAIRSPRING_EVENT line is one write(2), and the result line is exactly one write" events
else
  not_run B5 "each HAIRSPRING_EVENT line is one write(2)" "strace is not installed here."
fi
step B6 "the SDK's own tests (release) and the conformance tests against the reference parser" goldens
step B7 "the modes: wide differs from buggy only in the busy loop length" modes

say "## Evidence for the other checklist lines (captured from the built artifact above)"
say ""
evidence_steps

alignment_checklist \
  "One artifact per language, named \`$artifact\`; no separate racy and locked builds, no build flag choosing the mode." \
  "argv[1] is buggy, fixed or wide; anything else, or nothing, prints a usage line on stderr and exits 64." \
  "No read of the seed, /proc, the CPU count or the clock outside the runtime and the random source."

finish_record
exit $failed
