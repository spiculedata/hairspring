#!/bin/sh
# Summarise results/m1d into one table (stdout; redirect to results/m1d/SUMMARY.txt). Counts only; no reading of
# the numbers beyond the labels. A cell whose directory is missing is NOT-RUN. A race cell is "reported" when any
# run's stderr contains "WARNING: DATA RACE", else "clean" (ran, no report).
#
# columns: toolchain, cell, runs, runs_with_violation (property falses > 0 in the protocol report), property_falses
# (sum), foreign_next (sum), nonzero_exits (exit status other than 0 for native cells; 0 or 66 for race cells), races
# (runs whose stderr has a race report), race_reports (sum of reports), status
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
res=${M1D_RESULTS:-$here/results/m1d}
toolchains="go1.24.5 go1.24.6 go1.24.5-fixonly"
cells="native-yield-gmp2 native-yield-gmpdef native-std-gmp2 native-std-gmpdef race-std race-yield"

echo "M1d summary: informational, native macOS darwin/arm64; race cells are labelled 'race detector on macOS'."
echo "Not Hermit. Not the engine. Counts only."
echo "Not comparable with any Hermit count (different host, different -q, no scheduler)."
echo
echo "Columns: runs = runs in the cell; viol_runs = runs whose protocol report has the property site false at least"
echo "once; falses = the property site's false evaluations, summed over the runs; foreign_next = driver Next calls"
echo "made while another query's Scan was in flight (a driver-side overlap counter, not a property result), summed;"
echo "nonzero = runs with an unexpected exit status; races = runs whose stderr has a race-detector report;"
echo "reports = race-detector reports, summed; status = ran, or for race cells ran-and-clean / ran-and-reported."
echo
echo "Cells: native = the plain build, GOMAXPROCS=2 (gmp2) or the default (gmpdef); race = built with the race"
echo "detector; std = the unassisted workload, yield = the assisted control."
echo
printf '%-18s %-18s %5s %9s %10s %12s %8s %6s %8s  %s\n' toolchain cell runs viol_runs falses foreign_next nonzero races reports status
for t in $toolchains; do
  for c in $cells; do
    d="$res/$t/$c"
    if [ ! -d "$d" ]; then
      printf '%-18s %-18s %5s %9s %10s %12s %8s %6s %8s  %s\n' "$t" "$c" - - - - - - - NOT-RUN
      continue
    fi
    runs=0; viol_runs=0; falses=0; fnext=0; nonzero=0; races=0; reports=0
    for e in "$d"/run-*.err; do
      base=${e%.err}
      runs=$((runs + 1))
      f=$(grep '"id":"go.sql.scan_returns_own_row"' "$e" | grep '"kind":"assert"' | sed -n 's/.*"falses": *\([0-9][0-9]*\).*/\1/p' | head -n 1)
      f=${f:-0}
      falses=$((falses + f))
      [ "$f" -gt 0 ] && viol_runs=$((viol_runs + 1))
      n=$(sed -n 's/.*foreign_next= *\([0-9][0-9]*\).*/\1/p' "$base.out" | head -n 1)
      fnext=$((fnext + ${n:-0}))
      code=$(cat "$base.exit")
      case "$c" in
        race-*) [ "$code" != 0 ] && [ "$code" != 66 ] && nonzero=$((nonzero + 1)) ;;
        *) [ "$code" != 0 ] && nonzero=$((nonzero + 1)) ;;
      esac
      r=$(grep -c 'WARNING: DATA RACE' "$e" || true)
      reports=$((reports + r))
      [ "$r" -gt 0 ] && races=$((races + 1))
    done
    status=ran
    case "$c" in
      race-*) if [ "$races" -gt 0 ]; then status=ran-and-reported; else status=ran-and-clean; fi ;;
    esac
    printf '%-18s %-18s %5d %9d %10d %12d %8d %6d %8d  %s\n' "$t" "$c" "$runs" "$viol_runs" "$falses" "$fnext" "$nonzero" "$races" "$reports" "$status"
  done
done
