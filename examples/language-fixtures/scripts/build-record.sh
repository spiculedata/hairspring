#!/bin/sh
# Generates the BUILD-RECORD of one fixture artifact: checks B1 to B7 of the fixture contract, in one Markdown file.
#
#   build-record.sh <c|cxx-clang|cxx-gxx|go> [out-file]
#
# Run it on Linux x86-64, in the pinned build image (build-record-docker.sh does that). It builds the artifact twice,
# disassembles it, runs the seccomp self-test, runs the native self-check 50 times per mode on at least two CPUs,
# runs strace on one buggy run, runs the SDK's own tests and prints the source of the modes. It then captures the
# evidence for the rest of the alignment checklist from the built artifact (usage errors, the start barrier, one run's
# event lines, the ids, a stdout line, the exit codes, a grep of the source). Every step is recorded as PASS, FAIL or
# NOT-RUN with its output. The Markdown ends with the alignment checklist, each line ticked only when the evidence that
# proves it passed (record-lib.sh). Nothing here runs Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: CC / CXX / SDK_CC override the toolchain (defaults: gcc-13, g++-13 / clang++-18, gcc-13 / clang-18),
# IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARTIFACT_DIR receives a copy of the pinned
# binary and its SHA256SUMS (the workflow uploads it next to the record). The producer fields come from the GitHub
# Actions environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_*, evidence_*
# and barrier_sources variables are the interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
  exit 64
fi
variant=$1
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
runs=${RUNS:-50}
build=build/record
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

case "$variant" in
  c)
    dir=c lang=c artifact=at-c
    tool=${CC:-gcc-13}
    make_args="CC=$tool BUILD=$build"
    spin_re='<(lang_at_spin)>:' worker_re='<(worker)>:'
    barrier_sources=$root/c/src/lost_update.c
    source_files="$root/c/src/lost_update.c $root/common/lang_at.h"
    ;;
  cxx-clang)
    dir=cpp lang=cxx artifact=at-cxx-clang
    tool=${CXX:-clang++-18}
    SDK_CC=${SDK_CC:-clang-18}
    make_args="CXX=$tool SDK_CC=$SDK_CC BUILD=$build"
    spin_re='<(_ZL12lang_at_spinmj)>:' worker_re='<(_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode)>:'
    barrier_sources=$root/cpp/src/lost_update.cc
    source_files="$root/cpp/src/lost_update.cc $root/common/lang_at.h"
    ;;
  cxx-gxx)
    dir=cpp lang=cxx artifact=at-cxx-gxx
    tool=${CXX:-g++-13}
    SDK_CC=${SDK_CC:-gcc-13}
    make_args="CXX=$tool SDK_CC=$SDK_CC BUILD=$build"
    spin_re='<(_ZL12lang_at_spinmj)>:' worker_re='<(_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode)>:'
    barrier_sources=$root/cpp/src/lost_update.cc
    source_files="$root/cpp/src/lost_update.cc $root/common/lang_at.h"
    ;;
  go)
    dir=go lang=go artifact=at-go
    tool=${GO:-go}
    make_args="GO=$tool"
    spin_re='race\.spin>:' worker_re='race\.work>:'
    barrier_sources=$root/go/internal/race/race.go
    source_files="$root/go/main.go $root/go/internal/race/race.go"
    ;;
  *)
    echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
    exit 64
    ;;
esac
out=${2:-$root/build/record-$variant.md}
mkdir -p "$(dirname "$out")" || exit 1
fixtures=$root/$dir
if [ "$variant" = go ]; then
  build_dir=$fixtures/build/linux-amd64
  make_target=release
else
  build_dir=$fixtures/$build
  make_target=all
fi
binary=$build_dir/$artifact
GO_PINNED=${GO_PINNED:-go1.27.3}

# The make arguments are separate words on purpose.
# shellcheck disable=SC2086
build_twice() {
  echo "Build flags include -g (debug info); the sources are mapped to relative paths (-ffile-prefix-map, -trimpath)"
  echo "so the sha256 does not depend on the directory the tree is built in."
  rm -rf "$build_dir" "$fixtures/$build-second"
  GOCACHE=$(mktemp -d "$work/gocache.XXXXXX") make -C "$fixtures" "$make_target" $make_args || return 1
  first=$(sha256sum "$binary" | cut -d' ' -f1)
  # The second build uses a fresh Go build cache, so every package is compiled again rather than taken from the first
  # build, and, for C and C++, a different build directory, which must not change the artifact.
  second_binary=$binary
  second_args=$make_args
  if [ "$lang" != go ]; then
    second_args="${make_args%BUILD=*}BUILD=$build-second"
    second_binary=$fixtures/$build-second/$artifact
  else
    rm -rf "$build_dir"
  fi
  GOCACHE=$(mktemp -d "$work/gocache.XXXXXX") make -C "$fixtures" "$make_target" $second_args >/dev/null || return 1
  second=$(sha256sum "$second_binary" | cut -d' ' -f1)
  echo "sha256 of $artifact, first build:  $first"
  echo "sha256 of $artifact, second build: $second"
  [ "$first" = "$second" ] || { echo "the two builds differ"; return 1; }
  echo "the two builds are equal"
}

versions() {
  uname -srm
  if [ "$lang" = go ]; then
    got=$("$tool" version) || return 1
    echo "$got"
    case "$got" in
      *" $GO_PINNED "*) echo "ok   the Go toolchain is the pinned $GO_PINNED" ;;
      *) echo "FAIL the Go toolchain is not the pinned $GO_PINNED"; return 1 ;;
    esac
  else
    "$tool" --version | head -1
    [ -z "${SDK_CC:-}" ] || { echo "SDK library compiler: $SDK_CC"; "$SDK_CC" --version | head -1; }
  fi
  ldd --version 2>/dev/null | head -1
  objdump --version | head -1
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
}

disassembly() {
  objdump -d --no-show-raw-insn "$binary" >"$work/dis.s" || return 1
  extract "$spin_re" "$work/dis.s" >"$work/spin.s"
  extract "$worker_re" "$work/dis.s" >"$work/worker.s"
  [ -s "$work/spin.s" ] && [ -s "$work/worker.s" ] || { echo "the busy loop or the worker was not found"; return 1; }
  cat "$work/spin.s"
  echo "--- worker, around every call to the busy loop (the compiler may emit one copy of the window per mode)"
  grep -n -B12 -A12 'call.*spin' "$work/worker.s"
  sh "$here/check-disasm.sh" loop "$work/spin.s" || { echo "no backward branch in the busy loop"; return 1; }
  echo "ok   the busy loop has a backward branch"
  # Every call to the busy loop must sit between a load and a store of the same address with nothing else between.
  # C and C++ must also have a copy of the window that is not behind a mutex call (the buggy path). Go's gate is an
  # interface call, so the unlocked path cannot be told apart by name there.
  unlocked=1
  [ "$lang" = go ] && unlocked=0
  UNLOCKED_PATH=$unlocked sh "$here/check-disasm.sh" window "$work/worker.s" || { echo "a call to the busy loop is not a load -> call -> store of one address"; return 1; }
  echo "ok   every call to the busy loop is load -> call -> store of the same address, with no other call or locked read-modify-write between"
  if [ "$lang" = go ]; then
    echo "note: in Go the counter store is an XCHG with a memory operand, an implicitly locked exchange. It is a separate"
    echo "      instruction from the load and is not a read-modify-write of the counter (no LOCK XADD), which is what the"
    echo "      contract requires of the window."
  fi
}

# The test-only binary is built with the fixture's own compiler, as C++ for the C++ variants. It is never pinned.
seccomp() {
  lang_flag=
  [ "$lang" = cxx ] && lang_flag="-x c++"
  # shellcheck disable=SC2086  # lang_flag is empty or two words on purpose
  "$tool" $lang_flag -O2 -g -Wall -Wextra -Werror -pthread -I"$root/common" -o "$work/seccomp_window" "$root/tests/seccomp_window.c" || return 1
  "$work/seccomp_window"
}

native() {
  case "$lang" in go) GOMAXPROCS=2; export GOMAXPROCS ;; esac
  LANG_AT_REQUIRE_PARALLEL=1 sh "$here/check-native.sh" "$lang" "$binary" "$runs"
}

events() {
  # -s 4096 so the whole line is in the trace. The SDKs write their report to the events descriptor: stderr, or a
  # private copy of it (the C SDK uses fd 200, the Go SDK fd 3).
  strace -f -s 4096 -e trace=write,writev -o "$work/strace.txt" "$binary" buggy >"$work/stdout.txt" 2>"$work/stderr.txt" || return 1
  # check-writes.sh joins calls that strace -f prints as "unfinished" and "resumed", and counts the protocol line
  # writes; that count must equal the protocol lines the program put on its events descriptor.
  lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
  sh "$here/check-writes.sh" "$work/strace.txt" >"$work/writes.txt" || { cat "$work/writes.txt"; return 1; }
  cat "$work/writes.txt"
  written=$(cut -d' ' -f1 "$work/writes.txt")
  echo "HAIRSPRING_EVENT lines on stderr: $lines; protocol line writes in the trace: $written"
  [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ]
}

goldens() {
  if [ "$lang" = go ]; then
    (cd "$root/../../sdks/go" && go test -count=1 ./...)
  else
    make -C "$root/../../sdks/c" conformance
  fi
}

modes() {
  if [ "$lang" = go ]; then
    sed -n '/^func ParseMode/,/^}/p' "$root/go/internal/race/race.go"
  else
    sed -n '/lang_at_modes\[\]/,/^};/p' "$root/common/lang_at.h"
  fi
}

# The grep of the fixture source for line 11 of the checklist.
forbidden_reads() {
  # shellcheck disable=SC2086  # source_files is a list of paths on purpose
  sh "$here/check-sources.sh" $source_files
}

work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init

SOURCE_COMMIT=${SOURCE_COMMIT:-$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record.sh $variant."
artifact_name=$artifact
artifact_sha="<not built>"
evidence_artifact=$binary

say "Reproducibility: -ffile-prefix-map covers the fixture's own sources only. The SDK library keeps the absolute"
say "directory it was compiled in, so artifacts built in different checkouts or images (a CI runner and a container)"
say "differ in sha256. The pinned artifact comes from one named producer: the one named above."
say ""
step B1 "toolchain, build command (in the log) and two builds with equal sha256" versions
step B1 "the build, twice" build_twice
artifact_sha=$(publish_artifact "$binary" "$artifact") || artifact_sha="<not built>"
step B2 "the busy loop survived and the window's load and store are separate instructions" disassembly
if [ "$lang" = go ]; then
  not_applicable B3 "seccomp self-test" "Not applicable to Go: its runtime makes system calls on other threads. The no-syscall window is a code-review statement: spin touches no memory and makes no call."
else
  say "B3 note: the self-test installs a seccomp-BPF filter that allows only exit, exit_group and rt_sigreturn"
  say "(the same approach as the SDK's own no-syscall test), not SECCOMP_MODE_STRICT, and runs the window with W = 1000."
  say ""
  step B3 "seccomp self-test (one window per iteration, only exit allowed)" seccomp
fi
say "Exit-3 paths: a closed stdout is exercised by B4 (exit 3 required). A failed thread creation cannot be injected"
say "without hook code in the artifact, so it is covered by review of the abandoned-run path only."
say ""
step B4 "$runs native runs of buggy and fixed, wide as a contract check (at least two CPUs required)" native
if command -v strace >/dev/null 2>&1; then
  step B5 "each HAIRSPRING_EVENT line is one write(2)" events
else
  not_run B5 "each HAIRSPRING_EVENT line is one write(2)" "strace is not installed here."
fi
if [ "$lang" != go ] && ! command -v cargo >/dev/null 2>&1; then
  not_run B6 "the SDK's conformance goldens" "cargo is not installed here. Run \`make -C sdks/c conformance\` on a host with cargo (the Mac or hosted CI) and paste its output."
else
  step B6 "the SDK's own tests and conformance goldens" goldens
fi
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
