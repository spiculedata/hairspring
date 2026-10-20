#!/bin/sh
# run-selftests.sh [dpos|dneg|dstall|all]
#
# The Mac detector self-tests of condlock (native macOS pthread: informational evidence, "on macOS, not glibc, not this
# engine"). Raw outputs of every run are kept as files with sha256:
#
#   <results>/MANIFEST, SUMMARY.txt, SHA256SUMS
#   <results>/<cell>/run-NNNN.{out,err,exit,class}     cells: dpos, dneg, dstall-leak, dstall-barrier, dstall-holdg
#   <results>/raw/<cell>.tar.gz, TARBALLS.sha256       packed by pack-results.sh at the end; member counts in MANIFEST
#
# D-pos:   planted drop-signal builds (k cycled over 1 2 5 20 50), must fire in every run where the plant applied.
# D-neg:   the counted build, healthy; nothing may fire; the run is vacuous unless the watchdog ticked (mean >= 3).
# D-stall: planted leak, barrier and hold-G builds; must end in STALL (exit 4) with the property site true
#          (subtypes L, B and M; the hold-G cell is D-stall-M and exercises the watchdog's never-block-untimed path).
#
# Results are evidence: this script never writes into an existing results directory.
#   CL_BUILD     build dir with condlock and planted/ (default: ../build)
#   M1_RESULTS   new results directory (default: results/m1-mac-<UTC stamp>)
#   DPOS_RUNS 200, DNEG_RUNS 1000, DSTALL_RUNS 100 (per plant), R_POS, R_NEG, R_STALL, NWORKERS 8,
#   TICK_MS 20, QUORUM 10, RUN_CAP 60 (seconds, a run over it is killed: class TIMEOUT)
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
suite=${1:-all}
build=${CL_BUILD:-$here/build}
res=${M1_RESULTS:-$here/results/m1-mac-$(date -u +%Y%m%dT%H%M%SZ)}

dpos_runs=${DPOS_RUNS:-200}
dneg_runs=${DNEG_RUNS:-1000}
dstall_runs=${DSTALL_RUNS:-100}
r_pos=${R_POS:-200}
r_neg=${R_NEG:-5000}
r_stall=${R_STALL:-200}
nworkers=${NWORKERS:-8}
tick_ms=${TICK_MS:-20}
quorum=${QUORUM:-10}
run_cap=${RUN_CAP:-60}
ks="1 2 5 20 50"

case $suite in dpos | dneg | dstall | all) ;; *) echo "run-selftests.sh: unknown suite '$suite'" >&2; exit 64 ;; esac

# Never write into an existing directory: the plain mkdir below (no -p) is the atomic test.
if [ -e "$res" ]; then
  echo "run-selftests.sh: refusing to write into existing $res (set M1_RESULTS to a new directory)" >&2
  exit 1
fi
for b in condlock planted/condlock-drop-k1 planted/condlock-leak-k1 planted/condlock-barrier-k1 planted/condlock-holdg-k1; do
  [ -x "$build/$b" ] || { echo "run-selftests.sh: missing $build/$b (run make condlock selftest-bins)" >&2; exit 2; }
done
mkdir -p "$(dirname "$res")"
mkdir "$res"

sha() { shasum -a 256 "$1" | awk '{ print $1 }'; }

# pick_k <run index>: the k of this run, cycling through the fixed list before any run
pick_k() { echo "$ks" | awk -v i="$1" '{ print $(((i - 1) % NF) + 1) }'; }

# one_run <cell dir> <run number> <binary> <rounds>
one_run() {
  d=$1; num=$(printf '%04d' "$2"); bin=$3; rounds=$4
  env -i HOME="$HOME" PATH="/usr/bin:/bin" "$bin" -n "$nworkers" -r "$rounds" -tick-ms "$tick_ms" -q "$quorum" \
    >"$d/run-$num.out" 2>"$d/run-$num.err" &
  pid=$!
  ( sleep "$run_cap"; kill -KILL "$pid" 2>/dev/null ) 2>/dev/null &
  killer=$!
  code=0
  wait "$pid" || code=$?
  pkill -P "$killer" 2>/dev/null || true
  kill "$killer" 2>/dev/null || true
  wait "$killer" 2>/dev/null || true
  echo "$code" >"$d/run-$num.exit"
  sh "$here/scripts/classify.sh" "$code" "$d/run-$num.out" "$d/run-$num.err" >"$d/run-$num.class"
}

# cell <name> <runs> <binary pattern with %K%> <rounds>
cell() {
  name=$1; runs=$2; pattern=$3; rounds=$4
  d="$res/$name"
  mkdir "$d"
  i=1
  while [ "$i" -le "$runs" ]; do
    bin=$(echo "$pattern" | sed "s/%K%/$(pick_k "$i")/")
    one_run "$d" "$i" "$bin" "$rounds"
    i=$((i + 1))
  done
  echo "$name: $runs runs done"
}

{
  echo "condlock Mac self-tests (on macOS, not glibc, not this engine; informational)"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(uname -sm), $(sw_vers -productName) $(sw_vers -productVersion), ncpu $(sysctl -n hw.ncpu)"
  echo "load at start: $(uptime | sed 's/.*load averages*: //')"
  echo "compiler: $(cc --version | head -n 1)"
  echo "git: $(git -C "$here" rev-parse HEAD) (examples/glibc-condlock changed paths: $(git -C "$here" status --porcelain -- . ':!results' | wc -l | tr -d ' '))"
  echo "sdk tree: $(git -C "$here" rev-parse HEAD:sdks/c) (sdks/c changed paths: $(git -C "$here" status --porcelain -- ../../sdks/c | wc -l | tr -d ' '))"
  echo "suite: $suite"
  echo "sizes: dpos_runs=$dpos_runs dneg_runs=$dneg_runs dstall_runs=$dstall_runs r_pos=$r_pos r_neg=$r_neg r_stall=$r_stall"
  echo "flags: -n $nworkers -tick-ms $tick_ms -q $quorum; run cap ${run_cap}s; k list: $ks"
  echo "binaries (sha256):"
  echo "  $(sha "$build/condlock")  condlock"
  for p in "$build"/planted/condlock-*; do
    case $p in *.dSYM) continue ;; esac
    echo "  $(sha "$p")  planted/$(basename "$p")"
  done
} >"$res/MANIFEST"

case $suite in dpos | all) cell dpos "$dpos_runs" "$build/planted/condlock-drop-k%K%" "$r_pos" ;; esac
case $suite in dneg | all) cell dneg "$dneg_runs" "$build/condlock" "$r_neg" ;; esac
case $suite in
  dstall | all)
    cell dstall-leak "$dstall_runs" "$build/planted/condlock-leak-k%K%" "$r_stall"
    cell dstall-barrier "$dstall_runs" "$build/planted/condlock-barrier-k%K%" "$r_stall"
    cell dstall-holdg "$dstall_runs" "$build/planted/condlock-holdg-k%K%" "$r_stall"
    ;;
esac
echo "load at end: $(uptime | sed 's/.*load averages*: //')" >>"$res/MANIFEST"

sh "$here/scripts/pack-results.sh" "$res"
sh "$here/scripts/summarize-selftests.sh" "$res"
# shellcheck disable=SC2094 # find excludes SHA256SUMS, so the file being written is never read
(cd "$res" && find . -type f ! -name SHA256SUMS | sort | xargs shasum -a 256 >SHA256SUMS)
echo "results: $res"
