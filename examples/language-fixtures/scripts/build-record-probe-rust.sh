#!/bin/sh
# Generates the BUILD-RECORD of the Rust probe artifact probe-rust: the build and native checks of a std-only threaded
# program, in one Markdown file.
#
#   build-record-probe-rust.sh [out-file]
#
# Run it on Linux with the pinned Rust toolchain and the musl target installed (the manual CI job does). It builds
# probe-rust twice (two target directories) and requires equal sha256 (B1), searches the sources for forbidden reads, runs
# the program natively and checks only the format of its line and its exit status (B4), and runs strace -ff on one run to
# check that the line is one write (B5). The probe has no SDK, no property and no fixture window, so B2, B3, B6 and B7 do not
# apply, and there is no alignment checklist (that list is for the AT programs, see build-record-rust.sh). Nothing here runs
# Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: TARGET is the Rust target (default x86_64-unknown-linux-musl, the pinned static build), RUST_PINNED the
# toolchain version (default 1.99.1), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50),
# ARTIFACT_DIR receives a copy of the pinned binary and its SHA256SUMS. The producer fields come from the GitHub Actions
# environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_* variables are the
# interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
runs=${RUNS:-50}
target=${TARGET:-x86_64-unknown-linux-musl}
rust_pinned=${RUST_PINNED:-1.99.1}
out=${1:-$root/build/record-rust-probe.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

probe=$repo/sdks/rust/fixtures/probe
artifact="probe-rust"
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
target_one=$work/target-one
target_two=$work/target-two
binary=$target_one/$target/release/$artifact
# Paths of the checkout are mapped to /src, so the artifact does not depend on where the tree is built.
rustflags="--remap-path-prefix=$repo=/src"
# The six-field line and nothing else. No racy_final or timestamps: those are Python's four extra fields.
line_rx='^PROBE v1 lang=rust lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'

cargo_build() { # cargo_build <target-dir>
  RUSTFLAGS=$rustflags CARGO_TARGET_DIR=$1 cargo build --release --locked --target "$target" -p "$artifact" --bin "$artifact" --manifest-path "$repo/Cargo.toml"
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
  command -v strace >/dev/null 2>&1 && strace -V 2>&1 | head -1
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
  echo "build command: RUSTFLAGS='$rustflags' CARGO_TARGET_DIR=<dir> cargo build --release --locked --target $target -p $artifact --bin $artifact"
  echo "--- dependencies of $artifact (no dependencies, and not the SDK: only the crate itself may be listed)"
  deps=$(cargo tree --locked -p "$artifact" --target "$target" --manifest-path "$repo/Cargo.toml") || return 1
  echo "$deps"
  [ "$(printf '%s\n' "$deps" | wc -l | tr -d ' ')" -eq 1 ] || { echo "FAIL $artifact has dependencies"; return 1; }
  echo "ok   $artifact has no dependencies (std only)"
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

# The probe reads no seed, /proc, CPU count or environment, and no clock but Instant::now (its line carries the monotonic
# clock difference between the start and the end of the run: vtime_ns).
forbidden_reads() {
  sh "$here/check-sources.sh" --allow-instant "$probe/src" || return 1
  echo "ok   the probe reads none of them; its only clock read is Instant::now (vtime_ns)"
  grep -n 'Instant::now' "$probe"/src/*.rs
}

# B4, format only: every run exits 0 with an empty stderr and one stdout line of the six-field form. The distinct counts are
# information (natively the thread order and the random value vary from run to run; under Hermit they are a function of the
# seed). A usage error (any argument) exits 64 with nothing on stdout.
native() {
  i=0
  : >"$work/lines.txt"
  while [ "$i" -lt "$runs" ]; do
    "$binary" >"$work/out.txt" 2>"$work/err.txt"
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL run $i: exit status $rc, want 0"; return 1; }
    [ ! -s "$work/err.txt" ] || { echo "FAIL run $i: something on stderr:"; cat "$work/err.txt"; return 1; }
    [ "$(wc -l <"$work/out.txt" | tr -d ' ')" -eq 1 ] || { echo "FAIL run $i: stdout is not exactly one line:"; cat "$work/out.txt"; return 1; }
    grep -Eq "$line_rx" "$work/out.txt" || { echo "FAIL run $i: the line is not in the six-field form:"; cat "$work/out.txt"; return 1; }
    cat "$work/out.txt" >>"$work/lines.txt"
    i=$((i + 1))
  done
  echo "ok   $runs runs: exit 0, empty stderr, one line in the six-field form"
  tail -1 "$work/lines.txt"
  for name in lock_order arrival rand extra; do
    awk -v name="$name" '{ for (k = 1; k <= NF; k++) if (index($k, name "=") == 1 && !($k in seen)) { seen[$k] = 1; n++ } } END { print "information: " n " distinct " name " of " NR " runs" }' "$work/lines.txt"
  done
  for arg in x --switch=1.0 ""; do
    "$binary" "$arg" >"$work/out.txt" 2>"$work/err.txt"
    rc=$?
    [ "$rc" -eq 64 ] && [ ! -s "$work/out.txt" ] && [ -s "$work/err.txt" ] || { echo "FAIL the argument '$arg' is not a usage error (exit 64, usage line on stderr, nothing on stdout)"; return 1; }
  done
  echo "ok   any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace.
events() {
  strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$binary" >"$work/stdout.txt" 2>"$work/stderr.txt" || return 1
  cat "$work"/strace.* >"$work/strace-all.txt"
  sh "$here/check-probe-writes.sh" "$work/strace-all.txt"
}

# The probe's own unit and integration tests, in release, as the producer's toolchain runs them natively.
tests() {
  CARGO_TARGET_DIR=$work/target-test cargo test --locked --release -p "$artifact" --manifest-path "$repo/Cargo.toml"
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-probe-rust.sh."
artifact_name=$artifact
artifact_sha="<not built>"
record_no_checklist=1

say "Scope: this record covers probe-rust, a std-only program (no SDK, no property) used to check scheduling behaviour of Rust"
say "threads. It prints one \`PROBE v1\` line of six fields. Per the checks that apply to a probe, B1 and B4 (format only: the"
say "line matches the six-field form and the exit status is 0) are required; B5 is run as well. B2, B3, B6 and B7 belong to the"
say "AT programs and do not apply. The record for at-rust is produced by build-record-rust.sh."
say ""
say "Reproducibility: the checkout path is mapped to /src with RUSTFLAGS=--remap-path-prefix=<checkout>=/src. That flag is part"
say "of the build command (B1 prints it in full) and is not in a plain \`cargo build\`: an artifact built without it has another"
say "sha256. The Rust standard library is the toolchain's own. The pinned artifact comes from one named producer: the one"
say "named above."
say ""
step B1 "toolchain, build command, dependencies and two builds with equal sha256" versions
step B1 "the build, twice" build_twice
artifact_sha=$(publish_artifact "$binary" "$artifact") || artifact_sha="<not built>"
not_applicable B2 "disassembly of the busy loop and the window" "Not applicable: the probe has no fixture window to prove. Its racy counter exists to give the scheduler something to interleave, and its value is not part of the line."
not_applicable B3 "seccomp self-test" "Not applicable: the probe has no no-syscall window."
step F1 "forbidden reads: no seed, /proc, CPU count, environment or clock in the probe's sources (Instant::now allowed)" forbidden_reads
step B4 "$runs native runs: the line matches the six-field form and the exit status is 0; any argument is a usage error" native
if command -v strace >/dev/null 2>&1; then
  step B5 "the result line is exactly one write to stdout, and no protocol line exists" events
else
  not_run B5 "the result line is exactly one write" "strace is not installed here."
fi
not_applicable B6 "the SDK's conformance" "Not applicable: the probe does not use the SDK."
not_applicable B7 "wide differs from buggy only in the loop count" "Not applicable: the probe has no modes."
step T1 "the probe's own unit and integration tests (release)" tests

finish_record
exit $failed
