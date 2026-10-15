#!/bin/bash
# usage: pilot_worker.sh SYNC SEED -- Stage 0 pilot, mode 4, chaos 1ms, summary json captured
SYNC=$1; SEED=$2
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
t0=$(date +%s)
out=$(timeout 900 $H run --epoch=2026-01-01T00:00:00Z --chaos --max-timeslice=1000000 --summary-json=$W/pilot-s$SYNC-$SEED.json --sched-seed=$SEED -- $D/walwork7-3510200 $W/pilot-s$SYNC-$SEED.db 600 1 0 5 4 0 3 $SYNC 2>/dev/null); rc=$?
rm -f $W/pilot-s$SYNC-$SEED.db*
echo "PILOT sync=$SYNC seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) flags=[--chaos --max-timeslice=1000000] $(echo "$out" | head -1 | cut -c1-120)" >> $D/pilot.results
