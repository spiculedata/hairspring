#!/bin/bash
# usage: sweep4_worker.sh VER CFG SEED -- workload v3 (walwork4) under Hermit chaos; results to sweep4.results
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
case $CFG in
  ch2m)   F="--chaos --max-timeslice=2000000";;
  ch1m)   F="--chaos --max-timeslice=1000000";;
  ch500k) F="--chaos --max-timeslice=500000";;
  rnd)    F="--sched-heuristic=random";;
esac
t0=$(date +%s)
out=$(timeout ${TMO:-240} $H run --epoch=2026-01-01T00:00:00Z $F --sched-seed=$SEED -- $D/walwork4-$VER $W/$VER-$CFG-$SEED.db ${TXNS:-300} 1 0 20 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-110)" >> $D/sweep4.results
