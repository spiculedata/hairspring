#!/bin/sh
# Generates the BUILD-RECORD of the Python fixture package: checks B1 to B7 of the fixture contract, in one Markdown file.
#
#   build-record-python.sh [out-file]
#
# Python has nothing to compile, so the artifact is a package: the fixture (at_py.py), the probe (probe_py.py) and the SDK
# package (hairspring/), staged side by side. B1 stages it twice, lists every staged file with its sha256 and a tree hash,
# and requires the two listings to be equal. B2 (disassembly) and B3 (seccomp) do not apply to an interpreter; the window
# is checked by an `ast` test instead, and the sources are searched for forbidden reads. B4 runs the native self-check
# 50 times per mode on the staged package; B5 runs `strace -ff` on one buggy run and requires every protocol line and the
# result line to be one whole write(2). Every step is recorded as PASS, FAIL or NOT-RUN with its output; the Markdown ends
# with the alignment checklist to tick by hand. Nothing here runs Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: PYTHON (default python3; must be 3.12), IMAGE_DIGEST is copied into B1, RUNS is the native run count
# (default 50), PRODUCER names the host that produced the record.

# The check functions below are run through step, which shellcheck cannot see.
# shellcheck disable=SC2329
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
runs=${RUNS:-50}
python=${PYTHON:-python3}
py_pinned=${PY_PINNED:-3.12}
out=${1:-$root/build/record-python.md}
mkdir -p "$(dirname "$out")" || exit 1
: >"$out"
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

work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
stage1=$work/stage-one
stage2=$work/stage-two

versions() {
  uname -srm
  got=$("$python" --version 2>&1) || return 1
  echo "$got"
  case "$got" in
    "Python $py_pinned."*) echo "ok   the interpreter is Python $py_pinned" ;;
    *) echo "FAIL the interpreter is not Python $py_pinned"; return 1 ;;
  esac
  command -v strace >/dev/null 2>&1 && strace -V 2>&1 | head -1
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
}

# The package is staged twice, into different directories, and the listings (a sha256 per file, then the tree hash) must
# be equal. -B and PYTHONDONTWRITEBYTECODE keep bytecode out of the staged tree, which is also what the box runs.
staged_twice() {
  sh "$here/stage-python.sh" "$stage1" "$work/list-one.txt" || return 1
  sh "$here/stage-python.sh" "$stage2" "$work/list-two.txt" || return 1
  cat "$work/list-one.txt"
  cmp -s "$work/list-one.txt" "$work/list-two.txt" || { echo "the two stagings differ"; diff "$work/list-one.txt" "$work/list-two.txt"; return 1; }
  echo "the two stagings are equal (same files, same sha256 of every file, same tree hash)"
  grep -q ' at_py.py$' "$work/list-one.txt" && grep -q ' probe_py.py$' "$work/list-one.txt" && grep -q ' hairspring/__init__.py$' "$work/list-one.txt" ||
    { echo "the package is missing a file"; return 1; }
  if grep -q 'pycache\|\.pyc' "$work/list-one.txt"; then echo "bytecode was staged"; return 1; fi
}

# B2 does not apply (no compiler, no machine code). What the disassembly check proves for the others, the AST test proves
# here: the window is a read, the busy loop and a write, in that order, and nothing else.
window() {
  (cd "$root/python" && "$python" -m unittest test_at_py.ContractTests -v)
}

sources() {
  sh "$here/check-sources-python.sh" "$stage1/at_py.py" || return 1
  echo "ok   at_py.py reads no seed, /proc, CPU count, environment or clock"
  sh "$here/check-sources-python.sh" --allow-monotonic-ns "$stage1/probe_py.py" || return 1
  echo "ok   probe_py.py reads none of them either; its only clock read is time.monotonic_ns (its output carries the"
  echo "     monotonic clock before and after the run: vtime_ns, t_start_ns, t_end_ns)"
  grep -n 'monotonic_ns' "$stage1/probe_py.py"
}

native() {
  run=$work/run
  rm -rf "$run"
  cp -R "$stage1" "$run" || return 1
  cp "$root/python/check_native.py" "$run/check_native.py" || return 1
  (cd "$run" && "$python" -B check_native.py --python "$python" --runs "$runs" --switch-arm) || return 1
  (cd "$run" && "$python" -B check_native.py --python "$python" --runs 10 --control) || return 1
  echo "--- the probe prints one PROBE v1 line with the added fields, and exits 0"
  for arg in "" "--switch=1.0"; do
    # shellcheck disable=SC2086  # arg is empty or one word on purpose
    line=$(cd "$run" && "$python" -I -B probe_py.py $arg) || return 1
    echo "$line"
    printf '%s\n' "$line" | grep -Eq '^PROBE v1 lang=py lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16} racy_final=[0-9]{20} racy_expect=[0-9]{20} t_start_ns=[0-9]{20} t_end_ns=[0-9]{20}$' ||
      { echo "the probe line is not in the contract form"; return 1; }
  done
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace. The SDK writes its report to a private copy of stderr (fd 200); the result line goes to fd 1
# with one os.write.
events() {
  (cd "$stage1" && strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$python" -I -B at_py.py buggy >"$work/stdout.txt" 2>"$work/stderr.txt") || return 1
  cat "$work"/strace.* >"$work/strace-all.txt"
  lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
  sh "$here/check-writes.sh" "$work/strace-all.txt" >"$work/writes.txt" || { cat "$work/writes.txt"; return 1; }
  cat "$work/writes.txt"
  written=$(cut -d' ' -f1 "$work/writes.txt")
  echo "HAIRSPRING_EVENT lines on stderr: $lines; protocol line writes in the trace: $written"
  [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ]
}

goldens() {
  (cd "$repo/sdks/python" && "$python" -m unittest discover -s tests -t . -q) || return 1
  HS_REQUIRE_PYTHON=1 HS_PYTHON="$python" cargo test --manifest-path "$repo/Cargo.toml" -p hairspring-props --test python_sdk
}

modes() {
  sed -n '/^MODES = {/,/^}/p' "$stage1/at_py.py"
  sed -n '/^def racy_iterations/,/hs.work(1)/p' "$stage1/at_py.py"
}

say "# BUILD-RECORD: at_py.py (the Python fixture package)"
say ""
source_commit=${SOURCE_COMMIT:-$(git -C "$repo" rev-parse HEAD 2>/dev/null || echo unknown)}
say "Generated by scripts/build-record-python.sh. Source tree: \`$source_commit\`."
say ""
say "Producer: ${PRODUCER:-<TO BE FILLED: which host or image produced this record>}"
say ""
say "The artifact is a package, not one module: \`at_py.py\`, \`probe_py.py\` and the \`hairspring/\` directory, staged side by"
say "side. The tree hash in B1 identifies all of it. There is no compiler: the file contents are the build."
say ""
step B1 "toolchain" versions
step B1 "the package staged twice: every file's sha256 and the tree hash, equal both times" staged_twice
not_run B2 "disassembly of the busy loop and the window" "Not applicable: there is no machine code. The window (a read, the busy loop, a write, nothing else) is checked on the syntax tree by the next step, and the loop is Python bytecode that cannot be optimised away."
step B2 "the window on the syntax tree (read, busy loop, write; nothing between; the SDK is not called inside it)" window
not_run B3 "seccomp self-test" "Not applicable to Python: the interpreter allocates in the window (big-integer arithmetic, range, a call frame) and its runtime makes system calls on other threads. The no-syscall window is a code-review statement: the window is the three statements checked in B2 and the busy loop, none of which calls the operating system."
step B3 "forbidden reads: no seed, /proc, CPU count, environment or clock in the fixture or the probe" sources
say "Exit-3 paths: a closed stdout is exercised by the fixture's own tests (exit 3 required, no traceback, end.status 3). A failed"
say "thread start cannot be injected without hook code in the artifact, so it is covered by review of the abort path only."
say ""
step B4 "$runs native runs of buggy and fixed, wide as a contract check, a negative control, and the probe line (no second CPU is needed: the interpreter lock switches threads on one CPU)" native
if command -v strace >/dev/null 2>&1; then
  step B5 "each HAIRSPRING_EVENT line is one write(2), and the result line is exactly one write" events
else
  not_run B5 "each HAIRSPRING_EVENT line is one write(2)" "strace is not installed here."
fi
if ! command -v cargo >/dev/null 2>&1; then
  not_run B6 "the SDK's conformance" "cargo is not installed here. Run \`HS_REQUIRE_PYTHON=1 cargo test -p hairspring-props --test python_sdk\` on a host with cargo and paste its output."
else
  step B6 "the SDK's own tests and conformance against the reference parser" goldens
fi
step B7 "the modes: wide differs from buggy only in the busy loop length" modes

say "## Alignment checklist (tick each line after reading the evidence above)"
say ""
say "- [ ] 1. One artifact (the package, named by its tree hash), named \`at_py.py\`; no separate racy and locked builds, no flag choosing the mode."
say "- [ ] 2. argv[1] is buggy, fixed or wide (and an optional second argument --switch=1.0); anything else, or nothing, prints a usage line on stderr and exits 64."
say "- [ ] 3. T = 4, K = 1000, W = 1000, W_wide = 10000; wide differs from buggy only in the loop count (B7)."
say "- [ ] 4. Start barrier: each worker increments started, then waits until started == 4."
say "- [ ] 5. The window: load; spin; store; nothing else between them (B2)."
say "- [ ] 6. Main, after joining: always(final == T*K) with four witness values [final, 4000, 4, 1000], then sometimes(started == 4), each once."
say "- [ ] 7. Ids are \`py.counter_exact\` and \`py.all_started\`."
say "- [ ] 8. One stdout line in the contract form, one write; nothing else on stdout (B5)."
say "- [ ] 9. Exit status 0 whatever the property says; 3 for an internal invariant failure; 64 for a usage error."
say "- [ ] 10. B1 to B7 pass and the SDK's conformance passes."
say "- [ ] 11. No read of the seed, /proc, the CPU count or the clock outside the runtime and the random source (B3); the probe's only clock read is time.monotonic_ns."

echo "wrote $out"
exit $failed
