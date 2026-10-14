#!/usr/bin/env bash
# Build rrnewton/hermit (the maintained fork) and test forced determinism on a racy binary.
# Run as root on x86-64 Linux. Needs racy.c next to this script.
set -uo pipefail
cd "$(dirname "$0")"
RUNS=${RUNS:-100}
OUT=${OUT:-/root/hermit-fork-results.txt}
exec > >(tee "$OUT") 2>&1

echo "== diagnostics (per hermit issue #11) =="
uname -a; lscpu | grep -E "Model name|Vendor|Hypervisor|Flags" | cut -c1-160
echo "perf_event_paranoid=$(cat /proc/sys/kernel/perf_event_paranoid)"; systemd-detect-virt || true
echo 1 > /proc/sys/kernel/perf_event_paranoid
echo 0 > /proc/sys/kernel/yama/ptrace_scope 2>/dev/null || true
echo 0 > /proc/sys/kernel/apparmor_restrict_unprivileged_userns 2>/dev/null || true

echo; echo "== deps =="
export DEBIAN_FRONTEND=noninteractive
echo 2000 > /proc/sys/dev/raid/speed_limit_max 2>/dev/null || true   # keep RAID sync from starving apt
apt-get update -qq && apt-get install -y -qq build-essential pkg-config libunwind-dev liblzma-dev git curl >/dev/null 2>&1
command -v cargo >/dev/null || { curl -sSf https://sh.rustup.rs | sh -s -- -y >/dev/null 2>&1; }
. "$HOME/.cargo/env"
gcc -O1 -pthread racy.c -o racy

echo; echo "== build fork =="
[ -d /opt/hermit-fork ] || git clone -q --recurse-submodules https://github.com/rrnewton/hermit /opt/hermit-fork
cd /opt/hermit-fork && git log -1 --format='%h %ad %s' --date=short
cargo build --release --workspace 2>&1 | grep -E "^error|Finished|could not compile" | cut -c1-200 | head -8
H=/opt/hermit-fork/target/install_pkg/hermit; [ -x "$H" ] || H=/opt/hermit-fork/target/release/hermit
cd - >/dev/null
mkdir -p /opt/ht && cp racy /opt/ht/racy && chmod 755 /opt/ht /opt/ht/racy   # not /tmp (hermit replaces it) or $HOME
R=/opt/ht/racy
line() { grep -E "^[0-9a-f]{16} " | tail -1; }
distinct() { grep -E "^[0-9a-f]{16} " | sort | uniq -c | sort -rn | awk '{print $1"x "$2" "$3}' | head -8; }

echo; echo "== smoke: hermit run -- /bin/echo hello =="
$H run -- /bin/echo hello 2>&1 | tail -3 | cut -c1-200
echo; echo "== native racy x$RUNS (baseline) =="
for _ in $(seq $RUNS); do $R; done | distinct
echo; echo "== hermit run racy x$RUNS (want exactly 1 distinct) =="
for _ in $(seq $RUNS); do $H run -- $R 2>/dev/null | line; done | distinct
echo; echo "== sched-seed exploration: 20 seeds (want several distinct), then each of 5 seeds twice (want SAME) =="
for s in $(seq 1 20); do $H run --sched-seed=$s -- $R 2>/dev/null | line; done | distinct
for s in 1 2 3 4 5; do a=$($H run --sched-seed=$s -- $R 2>/dev/null | line); b=$($H run --sched-seed=$s -- $R 2>/dev/null | line); echo "seed $s: ${a:0:16} ${b:0:16} $([ -n "$a" ] && [ "$a" = "$b" ] && echo SAME || echo DIFF)"; done
echo; echo "results saved to $OUT"
