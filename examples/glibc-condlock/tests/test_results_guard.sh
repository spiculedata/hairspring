#!/bin/sh
# run-selftests.sh must never write into an existing results directory, and a tiny real run must produce the files
# a reader needs. Usage: test_results_guard.sh <build dir>
set -u

build=${1:?usage: test_results_guard.sh <build dir>}
here=$(cd "$(dirname "$0")/.." && pwd)
run=$here/scripts/run-selftests.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/condlock-guard.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
checks=0
failed=0

check() {
  name=$1; shift
  checks=$((checks + 1))
  if "$@"; then echo "ok   $name"; else echo "FAIL $name"; failed=$((failed + 1)); fi
}

tiny() { # tiny <results dir>: a very small real run of every suite
  CL_BUILD=$build M1_RESULTS=$1 DPOS_RUNS=5 DNEG_RUNS=3 DSTALL_RUNS=2 R_POS=60 R_NEG=20 R_STALL=60 sh "$run" all
}

# 1. an existing directory is refused and left byte-for-byte alone
mkdir "$tmp/existing"
echo "precious" >"$tmp/existing/evidence.txt"
before=$(shasum -a 256 "$tmp/existing/evidence.txt")
refused() { ! tiny "$tmp/existing" >"$tmp/refuse.out" 2>"$tmp/refuse.err"; }
check "existing results dir is refused (non-zero exit)" refused
check "refusal says why" grep -q 'refusing to write into existing' "$tmp/refuse.err"
untouched() { [ "$(shasum -a 256 "$tmp/existing/evidence.txt")" = "$before" ] && [ "$(ls "$tmp/existing")" = "evidence.txt" ]; }
check "existing results dir is untouched" untouched

# 2. a fresh directory works and is complete
fresh() { tiny "$tmp/fresh" >"$tmp/fresh.out" 2>"$tmp/fresh.err"; }
check "fresh results dir: tiny run succeeds" fresh
check "MANIFEST written" test -s "$tmp/fresh/MANIFEST"
check "SUMMARY written" test -s "$tmp/fresh/SUMMARY.txt"
check "SHA256SUMS written" test -s "$tmp/fresh/SHA256SUMS"
check "raw per-run files kept (dpos run 1: out, err, exit, class)" sh -c "
  d='$tmp/fresh/dpos'; test -f \$d/run-0001.out && test -f \$d/run-0001.err && test -f \$d/run-0001.exit && test -f \$d/run-0001.class"
check "all three stall cells ran (leak, barrier, holdg)" sh -c "
  test -f '$tmp/fresh/dstall-leak/run-0001.class' && test -f '$tmp/fresh/dstall-barrier/run-0001.class' &&
  test -f '$tmp/fresh/dstall-holdg/run-0001.class'"
members_ok() { # every tarball has a MANIFEST line with its real member count
  for t in "$tmp"/fresh/raw/*.tar.gz; do
    cell=$(basename "$t" .tar.gz)
    n=$(tar -tzf "$t" | wc -l | tr -d ' ')
    grep -q "^  raw/$cell.tar.gz: $n members\$" "$tmp/fresh/MANIFEST" || return 1
  done
}
check "tarballs packed by the runner" test -s "$tmp/fresh/TARBALLS.sha256"
check "MANIFEST lists each tarball's member count" members_ok
check "SUMMARY has a D-stall-M verdict" grep -q 'D-stall (holdg) verdict' "$tmp/fresh/SUMMARY.txt"
check "SHA256SUMS verifies" sh -c "cd '$tmp/fresh' && shasum -a 256 -c SHA256SUMS >/dev/null"
check "SUMMARY carries the macOS label" grep -q 'on macOS, not glibc, not this engine' "$tmp/fresh/SUMMARY.txt"

# 3. the same fresh directory cannot be reused
reuse() { ! tiny "$tmp/fresh" >/dev/null 2>&1; }
check "a finished results dir is refused too" reuse

echo "$checks checks, $failed failed"
[ "$failed" = 0 ]
