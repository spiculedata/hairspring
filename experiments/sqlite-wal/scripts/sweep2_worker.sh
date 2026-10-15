#!/bin/bash
# usage: sweep2_worker.sh VER CFG SEED   -- walwork2 under Hermit; results appended to sweep2.results
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
RND="--sched-heuristic=random"; E=""; BIN=walwork2
case $CFG in
  plain)  BIN=walwork2;;
  yield)  BIN=walwork2-yield;;
  d50)    BIN=walwork2-delay; E="--env HS_DELAY_US=50";;
  d500)   BIN=walwork2-delay; E="--env HS_DELAY_US=500";;
  d5000)  BIN=walwork2-delay; E="--env HS_DELAY_US=5000";;
  chaos)  BIN=walwork2-yield; RND="--chaos";;
esac
t0=$(date +%s)
out=$(timeout ${TMO:-200} $H run --epoch=2026-01-01T00:00:00Z $RND $E --sched-seed=$SEED -- $D/$BIN-$VER $W/$VER-$CFG-$SEED.db ${TXNS:-300} 1 1 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-110)" >> $D/sweep2.results
