#!/usr/bin/env bash
# Runs each test N times natively and under hermit; reports distinct outputs and failures.
H=${H:-/opt/hermit-fork/target/release/hermit}; D=/opt/ht; N=${N:-20}
EPOCH="--epoch=2026-01-01T00:00:00Z"
echo 1 > /proc/sys/kernel/perf_event_paranoid
summ() { # label file
  awk -v l="$1" '{c[$1]++; if($2!="ok")f++} END{d=0; for(k in c)d++; printf "%-34s distinct=%-3d failures=%d/%d\n", l, d, f+0, NR}' "$2"
}
run_n() { # outfile cmd...
  out=$1; shift; : > "$out"
  for _ in $(seq $N); do o=$("$@" 2>/dev/null); echo "$o" | grep -q '^DONE' && st=ok || st=FAIL; echo "$(echo "$o" | md5sum | cut -c1-8) $st" >> "$out"; done
}
for t in t1_syscalls t2_net t3_http t4_procs; do
  run_n /tmp/nat.$t python3 $D/$t.py
  run_n /tmp/her.$t $H run $EPOCH -- /usr/bin/python3 $D/$t.py
  summ "$t NATIVE" /tmp/nat.$t; summ "$t HERMIT" /tmp/her.$t
done
echo; echo "== t1 per-key: distinct values over $N runs (native | hermit)"
for m in nat her; do : > /tmp/keys.$m; done
for i in $(seq $N); do python3 $D/t1_syscalls.py >> /tmp/keys.nat 2>/dev/null; $H run $EPOCH -- /usr/bin/python3 $D/t1_syscalls.py >> /tmp/keys.her 2>/dev/null; done
for k in pid ppid time_s time_ns_low busy_ms sleep_ms urandom random uuid4 hash_abc hostname cpu_count listdir dir_ino dir_mtime; do
  n=$(grep "^$k=" /tmp/keys.nat | sort -u | wc -l); h=$(grep "^$k=" /tmp/keys.her | sort -u | wc -l)
  printf "  %-14s native=%-3s hermit=%s\n" $k $n $h
done
echo; echo "== sample hermit output (one run)"; $H run $EPOCH -- /usr/bin/python3 $D/t4_procs.py 2>&1 | grep -v "^hermit:" | head -5
echo; echo "== timing, t3_http x5: native vs hermit"
time (for _ in 1 2 3 4 5; do python3 $D/t3_http.py >/dev/null 2>&1; done)
time (for _ in 1 2 3 4 5; do $H run $EPOCH -- /usr/bin/python3 $D/t3_http.py >/dev/null 2>&1; done)
echo; echo "== t5 external network (informational)"
echo -n "native : "; curl -sS -m 8 -o /dev/null -w '%{http_code}\n' https://example.com 2>&1 | tail -1
echo -n "hermit : "; $H run $EPOCH -- /usr/bin/curl -sS -m 8 -o /dev/null -w '%{http_code}\n' https://example.com 2>&1 | grep -v "^hermit:" | tail -2
echo -n "hermit dns: "; $H run $EPOCH -- /usr/bin/getent hosts example.com 2>&1 | grep -v "^hermit:" | tail -2
