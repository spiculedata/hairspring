#!/bin/bash
# usage: sweep2_worker.sh VER CFG SEED   -- walwork2 under Hermit; results appended to sweep3.results
VER=$1; CFG=$2; SEED=$3
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
RND="--sched-heuristic=random"; E=""; BIN=walwork3
case $CFG in
  plain)  BIN=walwork3;;
  yield)  BIN=walwork3-yield;;
  d50)    BIN=walwork3-delay; E="--env HS_DELAY_US=50";;
  d500)   BIN=walwork3-delay; E="--env HS_DELAY_US=500";;
  d5000)  BIN=walwork3-delay; E="--env HS_DELAY_US=5000";;
  sy50)   BIN=walwork3-yield; RND="--sched-heuristic=stickyrandom --sched-sticky-random-param=0.5";;
  sy90)   BIN=walwork3-yield; RND="--sched-heuristic=stickyrandom --sched-sticky-random-param=0.9";;
  sy99)   BIN=walwork3-yield; RND="--sched-heuristic=stickyrandom --sched-sticky-random-param=0.99";;
  ps90)   BIN=walwork3; RND="--sched-heuristic=stickyrandom --sched-sticky-random-param=0.9";;
  ps99)   BIN=walwork3; RND="--sched-heuristic=stickyrandom --sched-sticky-random-param=0.99";;
  chaos)  BIN=walwork3-yield; RND="--chaos";;
esac
t0=$(date +%s)
out=$(timeout ${TMO:-200} $H run --epoch=2026-01-01T00:00:00Z $RND $E --sched-seed=$SEED -- $D/$BIN-$VER $W/$VER-$CFG-$SEED.db ${TXNS:-300} 1 1 1 2>/dev/null); rc=$?
rm -f $W/$VER-$CFG-$SEED.db*
echo "RESULT ver=$VER cfg=$CFG seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) $(echo "$out" | head -1 | cut -c1-110)" >> $D/sweep3.results
