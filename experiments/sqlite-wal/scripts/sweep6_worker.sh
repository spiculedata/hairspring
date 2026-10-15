#!/bin/bash
# usage: sweep6_worker.sh VER CFG SEED -- gated workload v5 (walwork6, mode 3) under Hermit chaos; results to sweep6.results
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
BIN=walwork6; E=""
case $CFG in
  ctrl)  F="--chaos --max-timeslice=1000000"; BIN=walwork6-delay; E="--env HS_DELAY_US=100000";;
  g1m)   F="--chaos --max-timeslice=1000000";;
  g500k) F="--chaos --max-timeslice=500000";;
  g250k) F="--chaos --max-timeslice=250000";;
  g1ms)  F="--chaos --max-timeslice=1000000 --chaos-per-thread-slowdown";;
esac
t0=$(date +%s)
out=$(timeout ${TMO:-400} $H run --epoch=2026-01-01T00:00:00Z $F $E --sched-seed=$SEED -- $D/$BIN-$VER $W/$VER-$CFG-$SEED.db ${TXNS:-800} 1 0 5 3 0 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-120)" >> $D/sweep6.results
