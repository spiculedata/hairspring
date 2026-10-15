set -u
echo 1 > /proc/sys/kernel/perf_event_paranoid
H=/opt/hermit-fork/target/release/hermit
D=/opt/anthill-wal; cd $D
SEEDS=${SEEDS:-20}; TMO=${TMO:-120}
for v in 3510200 3510300; do
  echo "== hermit $v, $SEEDS seeds (3 writers x 60 txns, 1 checkpointer, timeout ${TMO}s)"
  t0=$(date +%s); viol=0; err=0; outs=""
  for s in $(seq 1 $SEEDS); do
    out=$(timeout $TMO $H run --epoch=2026-01-01T00:00:00Z --sched-heuristic=random --sched-seed=$s -- $D/walwork-$v $D/h-$v.db 3 60 1 2>/tmp/h.err); rc=$?
    case $rc in 0) ;; 1) viol=$((viol+1)); echo "seed $s VIOLATION: $out";; *) err=$((err+1)); echo "seed $s rc=$rc: $(tail -c 300 /tmp/h.err)";; esac
    echo "seed $s rc=$rc t=$(( $(date +%s)-t0 ))s"
    outs="$outs $(echo "$out" | md5sum | cut -c1-6)"
  done
  echo "hermit $v: violations=$viol errors=$err time=$(( $(date +%s)-t0 ))s distinct-outputs=$(echo $outs | tr ' ' '\n' | sort -u | wc -l)"
done
