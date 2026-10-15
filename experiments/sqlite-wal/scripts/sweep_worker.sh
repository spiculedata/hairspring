#!/bin/bash
# usage: sweep_worker.sh VER CFG SEED  (CFG defines flags + workload params)
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=$D/work; mkdir -p $W
case $CFG in
  c1) F="--sched-heuristic=random"; P="4 80 2 1";;
  c2) F="--sched-heuristic=random --max-timeslice=2000000"; P="3 60 2 4";;
  c3) F="--sched-heuristic=random --max-timeslice=500000"; P="4 80 2 1";;
  c4) F="--chaos"; P="4 80 2 1";;
  c5) F="--chaos --chaos-target-races"; P="4 80 2 1";;
  c6) F="--sched-heuristic=random"; P="2 150 3 1";;
esac
t0=$(date +%s)
out=$(timeout 150 $H run --epoch=2026-01-01T00:00:00Z $F --sched-seed=$SEED -- $D/walwork-$VER $W/$VER-$CFG-$SEED.db $P 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-100)" >> $D/sweep.results
