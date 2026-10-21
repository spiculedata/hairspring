#!/bin/sh
# Generates the BUILD-RECORD of one fixture artifact: checks B1 to B7 of the fixture contract, in one Markdown file.
#
#   build-record.sh <c|cxx-clang|cxx-gxx|go> [out-file]
#
# Run it on Linux x86-64, in the pinned build image (build-record-docker.sh does that). It builds the artifact twice,
# disassembles it, runs the seccomp self-test, runs the native self-check 50 times per mode on at least two CPUs,
# runs strace on one buggy run, runs the SDK's own tests and prints the source of the modes. Every step is recorded as
# PASS, FAIL or NOT-RUN with its output; the Markdown ends with the alignment checklist to tick by hand after reading
# the evidence. Nothing here runs Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: CC / CXX / SDK_CC override the toolchain (defaults: gcc-13, g++-13 / clang++-18, gcc-13 / clang-18),
# IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50).

# The check functions below are run through step, which shellcheck cannot see.
# shellcheck disable=SC2329
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

case "$variant" in
  c)
    dir=c lang=c artifact=at-c
    tool=${CC:-gcc-13}
    make_args="CC=$tool BUILD=$build"
    spin_re='<(lang_at_spin)>:' worker_re='<(worker)>:'
    ;;
  cxx-clang)
    dir=cpp lang=cxx artifact=at-cxx-clang
    tool=${CXX:-clang++-18}
    make_args="CXX=$tool SDK_CC=${SDK_CC:-clang-18} BUILD=$build"
    spin_re='<(_ZL12lang_at_spinmj)>:' worker_re='<(_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode)>:'
    ;;
  cxx-gxx)
    dir=cpp lang=cxx artifact=at-cxx-gxx
    tool=${CXX:-g++-13}
    make_args="CXX=$tool SDK_CC=${SDK_CC:-gcc-13} BUILD=$build"
    spin_re='<(_ZL12lang_at_spinmj)>:' worker_re='<(_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode)>:'
    ;;
  go)
    dir=go lang=go artifact=at-go
    tool=${GO:-go}
    make_args="GO=$tool"
    spin_re='race\.spin>:' worker_re='race\.work>:'
    ;;
  *)
    echo "usage: $0 <c|cxx-clang|cxx-gxx|go> [out-file]" >&2
    exit 64
    ;;
esac
out=${2:-$root/build/record-$variant.md}
mkdir -p "$(dirname "$out")" || exit 1
: >"$out"
fixtures=$root/$dir
if [ "$variant" = go ]; then
  binary=$fixtures/build/linux-amd64/$artifact
  make_target=release
else
  binary=$fixtures/$build/$artifact
  make_target=all
fi
failed=0

say() { printf '%s\n' "$*" >>"$out"; }

# step <id> <title> <command...>: runs the command, records its output in a fenced block and PASS/FAIL under the title.
step() {
  id=$1
  title=$2
  shift 2
  log=$(mktemp "${TMPDIR:-/tmp}/record.XXXXXX")
  "$@" >"$log" 2>&1
  rc=$?
  status=PASS
  if [ "$rc" -ne 0 ]; then
    status=FAIL
    failed=1
  fi
  say "### $id $title: $status"
  say ""
  say '```'
  say "\$ $*"
  cat "$log" >>"$out"
  say '```'
  say ""
  rm -f "$log"
  return "$rc"
}

not_run() { say "### $1 $2: NOT-RUN"; say ""; say "$3"; say ""; }

# extract <regex> <objdump-file>: the disassembly of the function whose header line matches.
extract() { awk -v re="$1" '$0 ~ re { on = 1 } on { print } on && /^$/ { exit }' "$2"; }

# The make arguments are separate words on purpose.
# shellcheck disable=SC2086
build_twice() {
  rm -rf "$fixtures/build"
  make -C "$fixtures" "$make_target" $make_args || return 1
  first=$(sha256sum "$binary" | cut -d' ' -f1)
  rm -rf "$fixtures/build"
  make -C "$fixtures" "$make_target" $make_args >/dev/null || return 1
  second=$(sha256sum "$binary" | cut -d' ' -f1)
  echo "sha256 of $artifact, first build:  $first"
  echo "sha256 of $artifact, second build: $second"
  [ "$first" = "$second" ] || { echo "the two builds differ"; return 1; }
  echo "the two builds are equal"
}

versions() {
  uname -srm
  "$tool" --version | head -1
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
  echo "--- worker, around the call to the busy loop"
  grep -n -B8 -A8 'call.*spin' "$work/worker.s"
  sh "$here/check-disasm.sh" loop "$work/spin.s" || { echo "no backward branch in the busy loop"; return 1; }
  echo "ok   the busy loop has a backward branch"
  sh "$here/check-disasm.sh" window "$work/worker.s" || { echo "a locked read-modify-write near the window, or no call"; return 1; }
  echo "ok   no locked read-modify-write around the window"
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
  strace -f -e trace=write -o "$work/strace.txt" "$binary" buggy >"$work/stdout.txt" 2>"$work/stderr.txt" || return 1
  # The SDKs write their report to the events descriptor: stderr, or a private copy of it (the C SDK uses fd 200).
  written=$(grep -Ec 'write\([0-9]+, "HAIRSPRING_EVENT' "$work/strace.txt")
  lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
  stdout_writes=$(grep -Ec 'write\(1, "LANGAT v1' "$work/strace.txt")
  echo "HAIRSPRING_EVENT lines on stderr: $lines; write calls that start one: $written"
  echo "write(1) calls that carry the result line: $stdout_writes"
  [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ] && [ "$stdout_writes" -eq 1 ]
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

work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

say "# BUILD-RECORD: $artifact"
say ""
say "Generated by scripts/build-record.sh $variant. Source tree: \`$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)\`."
say ""
step B1 "toolchain, build command (in the log) and two builds with equal sha256" versions
step B1 "the build, twice" build_twice
step B2 "the busy loop survived and the window's load and store are separate instructions" disassembly
if [ "$lang" = go ]; then
  not_run B3 "seccomp self-test" "Not applicable to Go: its runtime makes system calls on other threads. The no-syscall window is a code-review statement: spin touches no memory and makes no call."
else
  step B3 "seccomp self-test (one window per iteration, only exit allowed)" seccomp
fi
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

say "## Alignment checklist (tick each line after reading the evidence above)"
say ""
say "- [ ] 1. One artifact per language, named \`$artifact\`; no separate racy and locked builds, no build flag choosing the mode."
say "- [ ] 2. argv[1] is buggy, fixed or wide; anything else, or nothing, prints a usage line on stderr and exits 64."
say "- [ ] 3. T = 4, K = 1000, W = 1000, W_wide = 10000; wide differs from buggy only in the loop count (B7)."
say "- [ ] 4. Start barrier: each worker increments started, then waits until started == 4."
say "- [ ] 5. The window: load; spin; store; nothing else between them."
say "- [ ] 6. Main, after joining: always(final == T*K) with four witness values [final, 4000, 4, 1000], then sometimes(started == 4), each once."
say "- [ ] 7. Ids are \`$lang.counter_exact\` and \`$lang.all_started\`."
say "- [ ] 8. One stdout line in the contract form, one write; nothing else on stdout."
say "- [ ] 9. Exit status 0 whatever the property says; 3 for an internal invariant failure; 64 for a usage error."
say "- [ ] 10. B1 to B7 pass and the SDK's conformance goldens pass."
say "- [ ] 11. No read of the seed, /proc, the CPU count or the clock outside the runtime and the random source."

echo "wrote $out"
exit $failed
