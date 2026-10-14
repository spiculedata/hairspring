#!/usr/bin/env bash
# Runs on an x86 Linux host with PMU + KVM (bare metal). Tests spec assumptions U1-U4.
set -uo pipefail
cd "$(dirname "$0")"
echo "== host =="; uname -r; grep -m1 'model name' /proc/cpuinfo
echo "kvm: $(ls /dev/kvm 2>&1)"; echo "perf_event_paranoid: $(cat /proc/sys/kernel/perf_event_paranoid)"

echo; echo "== deps =="
apt-get update -qq && apt-get install -y -qq rr build-essential curl >/dev/null 2>&1
command -v cargo >/dev/null || { curl -sSf https://sh.rustup.rs | sh -s -- -y >/dev/null 2>&1; . "$HOME/.cargo/env"; }
cargo build --release -q 2>&1 | tail -3

echo; echo "== U4/Tier1: sim is deterministic on Linux x86 (expect same hash x2) =="
./target/release/e2_replay | head -3

echo; echo "== U2: does rr record at all on this CPU? =="
echo 1 > /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true
rr record -q ./target/release/e1_nondeterminism >/dev/null 2>&1 && echo "rr record: OK" || echo "rr record: FAILED"

echo; echo "== U2b: rr replay reproduces a racy run exactly? (100 record/replay pairs) =="
same=0; total=0
for _ in $(seq 1 100); do
  a=$(rr record -q ./target/release/e1_nondeterminism 2>&1 | md5sum)
  b=$(rr replay -a 2>&1 | md5sum)
  total=$((total+1)); [ "$a" = "$b" ] && same=$((same+1))
done
echo "replay == recorded output in $same/$total"

echo; echo "== rr chaos: does recording diversify thread interleavings? =="
for m in "" "--chaos"; do
  d=$(for _ in $(seq 1 50); do rr record -q $m ./target/release/e1_nondeterminism 2>&1 | sed -n 2p; done | sort -u | wc -l)
  echo "rr record ${m:-default}: $d distinct 'thread interleavings' lines in 50 runs"
done
echo; echo "U1 (Firecracker snapshot divergence) and U3 (Hermit) need follow-up scripts."
