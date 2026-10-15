#!/bin/bash
# usage: pilot5_worker.sh ROW SEED -- Stage 0e (mode 4, group 3, sync_off 1, 300 txns, 3.51.2, walwork8), isolated --tmp per run
ROW=$1; SEED=$2
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; T=/dev/shm/hs2/p5-$ROW-$SEED; J=/dev/shm/hs2/p5-$ROW-$SEED.json
BIN=walwork8-3510200; ENVF=""; VER=""; TMO=400; SK=""; GLOBAL=""
case $ROW in
  vdef) VER="--verify"; TMO=600;;
  v50)  SK="--skid-margin=50 --panic-on-rbc-overshoot"; VER="--verify"; TMO=600;;
  v25)  SK="--skid-margin=25 --panic-on-rbc-overshoot"; VER="--verify"; TMO=600;;
  s25)  SK="--skid-margin=25 --panic-on-rbc-overshoot";;
  jc2)  BIN=walwork8-jdelay-3510200; ENVF="--env HS_DELAY_BASE_US=100 --env HS_DELAY_SPREAD_US=300";;
  jdiag) BIN=walwork8-jdelay-3510200; ENVF="--env HS_DELAY_BASE_US=100 --env HS_DELAY_SPREAD_US=300"; GLOBAL="--log=info --log-file=/dev/shm/hs2/jc-diag.log";;
esac
F="--chaos --max-timeslice=1000000 $SK"
rm -rf $T; t0=$(date +%s)
out=$(timeout $TMO $H $GLOBAL run --epoch=2026-01-01T00:00:00Z $F $VER $ENVF --tmp=$T --summary-json=$J --sched-seed=$SEED -- $D/$BIN /tmp/w.db 300 1 0 5 4 0 3 1 2>$T.err); rc=$?
ov=$(grep -c "SKID_OVERSHOOT" $T.err 2>/dev/null)
st=$(python3 -c "import json;j=json.load(open('$J'));t=j['timeslice_stats'];print('syscalls=%d turns=%d slices=%d max_ns=%d'%(j['syscalls'],j['sched_turns'],t['count'],t['max_ns']))" 2>/dev/null)
echo "PILOT5 row=$ROW seed=$SEED rc=$rc secs=$(( $(date +%s)-t0 )) overshoot=$ov $st flags=[$F $VER $ENVF] $(echo "$out" | head -1 | cut -c1-110) $( [ $rc -ne 0 ] && tail -c 200 $T.err | tr '\n' ' ')" >> $D/pilot5.results
rm -rf $T $T.err $J
