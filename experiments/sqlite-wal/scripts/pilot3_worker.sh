#!/bin/bash
# usage: pilot3_worker.sh ROW SEED -- Stage 0c pilot (mode 4, group 3, sync_off 1, 300 txns, 3.51.2); results to pilot3.results
ROW=$1; SEED=$2
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; W=/dev/shm/hs2; mkdir -p $W
BIN=walwork7-3510200; ENVF=""; VER=""; TMO=400
case $ROW in
  ctrl15)  F="--chaos --max-timeslice=1000000"; BIN=walwork7-delay-3510200; ENVF="--env HS_DELAY_US=15000";;
  skid200) F="--chaos --max-timeslice=1000000 --skid-margin=200 --panic-on-rbc-overshoot"; [ "$SEED" -le 2 ] && VER="--verify";;
  skid50)  F="--chaos --max-timeslice=1000000 --skid-margin=50 --panic-on-rbc-overshoot"; [ "$SEED" -le 2 ] && VER="--verify";;
esac
t0=$(date +%s)
out=$(timeout $TMO $H run --epoch=2026-01-01T00:00:00Z $F $VER $ENVF --summary-json=$W/p3-$ROW-$SEED.json --sched-seed=$SEED -- $D/$BIN $W/p3-$ROW-$SEED.db 300 1 0 5 4 0 3 1 2>$W/p3-$ROW-$SEED.err); rc=$?
rm -f $W/p3-$ROW-$SEED.db*
ov=$(grep -c "SKID_OVERSHOOT" $W/p3-$ROW-$SEED.err 2>/dev/null)
st=$(python3 -c "import json;j=json.load(open('$W/p3-$ROW-$SEED.json'));t=j['timeslice_stats'];print('syscalls=%d turns=%d slices=%d max_ns=%d'%(j['syscalls'],j['sched_turns'],t['count'],t['max_ns']))" 2>/dev/null)
echo "PILOT3 row=$ROW seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) overshoot=$ov $st flags=[$F $VER $ENVF] $(echo "$out" | head -1 | cut -c1-110) $( [ $rc -ne 0 ] && tail -c 140 $W/p3-$ROW-$SEED.err | tr '\n' ' ')" >> $D/pilot3.results
