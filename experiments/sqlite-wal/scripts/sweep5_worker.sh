#!/bin/bash
# usage: sweep5_worker.sh VER CFG SEED -- workload v4 (walwork5) under Hermit chaos; results to sweep5.results
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
BIN=walwork5; E=""; M=1
case $CFG in
  a1m)    F="--chaos --max-timeslice=1000000"; M=1;;
  a1ms)   F="--chaos --max-timeslice=1000000 --chaos-per-thread-slowdown"; M=1;;
  a500k)  F="--chaos --max-timeslice=500000"; M=1;;
  b1m)    F="--chaos --max-timeslice=1000000"; M=2;;
  b1ms)   F="--chaos --max-timeslice=1000000 --chaos-per-thread-slowdown"; M=2;;
  ctrl)   F="--chaos --max-timeslice=1000000"; M=1; BIN=walwork5-delay; E="--env HS_DELAY_US=5000";;
esac
t0=$(date +%s)
out=$(timeout ${TMO:-300} $H run --epoch=2026-01-01T00:00:00Z $F $E --sched-seed=$SEED -- $D/$BIN-$VER $W/$VER-$CFG-$SEED.db ${TXNS:-400} 1 0 10 $M 4 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-110)" >> $D/sweep5.results
