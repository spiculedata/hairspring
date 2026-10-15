#!/bin/bash
# usage: pilot2_worker.sh ROW SEED -- Stage 0b pilot (mode 4, group 3, sync_off 1, 600 txns, 3.51.2); results to pilot2.results
ROW=$1; SEED=$2
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
BIN=walwork7-3510200; ENVF=""; VER=""
case $ROW in
  t250)     F="--chaos --max-timeslice=250000";;
  slow)     F="--chaos --max-timeslice=1000000 --chaos-per-thread-slowdown";;
  skid5000) F="--chaos --max-timeslice=1000000 --skid-margin=5000 --panic-on-rbc-overshoot"; [ "$SEED" -le 2 ] && VER="--verify";;
  skid1000) F="--chaos --max-timeslice=1000000 --skid-margin=1000 --panic-on-rbc-overshoot"; [ "$SEED" -le 2 ] && VER="--verify";;
  ctrl15)   F="--chaos --max-timeslice=1000000"; BIN=walwork7-delay-3510200; ENVF="--env HS_DELAY_US=15000";;
  base)     F="--chaos --max-timeslice=1000000";;
esac
t0=$(date +%s)
out=$(timeout 900 $H run --epoch=2026-01-01T00:00:00Z $F $VER $ENVF --summary-json=$W/p2-$ROW-$SEED.json --sched-seed=$SEED -- $D/$BIN $W/p2-$ROW-$SEED.db 600 1 0 5 4 0 3 1 2>$W/p2-$ROW-$SEED.err); rc=$?
rm -f $W/p2-$ROW-$SEED.db*
echo "PILOT2 row=$ROW seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) flags=[$F $VER $ENVF] $(echo "$out" | head -1 | cut -c1-110) $( [ $rc -ne 0 ] && tail -c 160 $W/p2-$ROW-$SEED.err | tr '\n' ' ')" >> $D/pilot2.results
