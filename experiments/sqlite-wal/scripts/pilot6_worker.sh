#!/bin/bash
# usage: pilot6_worker.sh KIND VER SEED TAG -- Stage 1/2-pilot worker (mode 4, group 3, sync_off 1, 300 txns), isolated --tmp
KIND=$1; VER=$2; SEED=$3; TAG=$4
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; N=p6-$KIND-$VER-$SEED-$TAG; T=/dev/shm/hs2/$N; J=/dev/shm/hs2/$N.json
BIN=walwork8-$VER; ENVF=""; VERIFY=""; TMO=400; TS=1000000; SK="--skid-margin=25 --panic-on-rbc-overshoot"
case $KIND in
  jc100d) BIN=walwork8-jdelay-$VER; ENVF="--env HS_DELAY_BASE_US=100 --env HS_DELAY_SPREAD_US=300"; SK="";;
  jc100)  BIN=walwork8-jdelay-$VER; ENVF="--env HS_DELAY_BASE_US=100 --env HS_DELAY_SPREAD_US=300";;
  jc550)  BIN=walwork8-jdelay-$VER; ENVF="--env HS_DELAY_BASE_US=550 --env HS_DELAY_SPREAD_US=350";;
  ver)    VERIFY="--verify"; TMO=600;;
  t250)   TS=250000; TMO=600;;
  t100)   TS=100000; TMO=600;;
esac
F="--chaos --max-timeslice=$TS $SK"
rm -rf $T; t0=$(date +%s)
out=$(timeout $TMO $H run --epoch=2026-01-01T00:00:00Z $F $VERIFY $ENVF --tmp=$T --summary-json=$J --sched-seed=$SEED -- $D/$BIN /tmp/w.db 300 1 0 5 4 0 3 1 2>$T.err); rc=$?
ov=$(grep -c "SKID_OVERSHOOT" $T.err 2>/dev/null)
st=$(python3 -c "import json;j=json.load(open('$J'));t=j['timeslice_stats'];print('syscalls=%d turns=%d slices=%d max_ns=%d'%(j['syscalls'],j['sched_turns'],t['count'],t['max_ns']))" 2>/dev/null)
echo "PILOT6 kind=$KIND ver=$VER seed=$SEED tag=$TAG rc=$rc secs=$(( $(date +%s)-t0 )) overshoot=$ov $st flags=[$F $VERIFY $ENVF] $(echo "$out" | head -1 | cut -c1-110) $( [ $rc -ne 0 ] && tail -c 200 $T.err | tr '\n' ' ')" >> $D/pilot6.results
rm -rf $T $T.err $J
