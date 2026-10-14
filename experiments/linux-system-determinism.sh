#!/usr/bin/env bash
# U2/U3 experiment: can an unmodified racy binary be made deterministic on this Linux host?
# Run as root on PMU-capable x86 Linux (bare metal). Safe to re-run. Prints a summary table.
set -uo pipefail
cd "$(dirname "$0")"
RUNS=${RUNS:-100}
distinct() { grep -E "^[0-9a-f]{16} " | sort | uniq -c | sort -rn | awk '{print $1"x "$2" "$3}' | head -5; }

echo "== host =="; uname -r; grep -m1 'model name' /proc/cpuinfo
echo "kvm: $(ls /dev/kvm 2>&1)"; echo "nproc: $(nproc)"
echo 1 > /proc/sys/kernel/perf_event_paranoid
echo "perf_event_paranoid: $(cat /proc/sys/kernel/perf_event_paranoid)"

echo; echo "== deps =="
apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq rr build-essential git curl pkg-config libssl-dev linux-tools-common >/dev/null 2>&1
gcc -O1 -pthread racy.c -o racy && echo "built racy"

echo; echo "== 0. baseline: native, $RUNS runs =="
for _ in $(seq $RUNS); do ./racy; done | distinct

echo; echo "== 1. rr record (default), $RUNS runs: distinct RECORDED outputs =="
for _ in $(seq $RUNS); do rr record ./racy 2>/dev/null | tail -1; done | distinct

echo; echo "== 2. rr record --chaos, $RUNS runs (diversifies schedules, for finding bugs) =="
for _ in $(seq $RUNS); do rr record --chaos ./racy 2>/dev/null | tail -1; done | distinct

echo; echo "== 3. rr replay reproduces a recording exactly? (20 record+replay pairs) =="
ok=0
for _ in $(seq 20); do
  a=$(rr record ./racy 2>/dev/null | tail -1); b=$(rr replay -a 2>/dev/null | tail -1)
  [ -n "$a" ] && [ "$a" = "$b" ] && ok=$((ok+1))
done; echo "replay == record in $ok/20"

echo; echo "== 4. Hermit: build =="
if ! command -v hermit >/dev/null; then
  command -v cargo >/dev/null || { curl -sSf https://sh.rustup.rs | sh -s -- -y >/dev/null 2>&1; . "$HOME/.cargo/env"; }
  [ -d /opt/hermit ] || git clone -q --depth 1 https://github.com/facebookexperimental/hermit /opt/hermit
  (cd /opt/hermit && cargo build --release -p hermit 2>&1 | tail -3)
  export PATH=/opt/hermit/target/release:$PATH
fi
if command -v hermit >/dev/null; then
  echo; echo "== 5. hermit run --strict, $RUNS runs: distinct outputs (want exactly 1) =="
  for _ in $(seq $RUNS); do hermit run --strict ./racy 2>/dev/null | tail -1; done | distinct
  echo; echo "== 6. hermit with different seeds (want different hashes, each reproducible) =="
  for s in 1 2 3; do echo "seed $s: $(hermit run --strict --seed=$s ./racy 2>/dev/null | tail -1) / $(hermit run --strict --seed=$s ./racy 2>/dev/null | tail -1)"; done
else
  echo "hermit not available (build failed) -> record as U3 = not currently buildable"
fi
