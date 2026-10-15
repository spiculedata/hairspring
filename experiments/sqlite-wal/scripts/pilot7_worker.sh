#!/bin/bash
# usage: pilot7_worker.sh KIND VER SEED TAG -- Stage 1c: double-yield writer (walwork9, yields=2), mode 4, group 3, sync_off 1, 300 txns
KIND=$1; VER=$2; SEED=$3; TAG=$4
H=/opt/hermit-fork/target/release/hermit; D=/opt/anthill-wal; N=p7-$KIND-$VER-$SEED-$TAG; T=/dev/shm/hs2/$N; J=/dev/shm/hs2/$N.json
BIN=walwork9-$VER; ENVF=""; TMO=400; TS=1000000; SK="--skid-margin=25 --panic-on-rbc-overshoot"
case $KIND in
  jc100y2) BIN=walwork9-jdelay-$VER; ENVF="--env HS_DELAY_BASE_US=100 --env HS_DELAY_SPREAD_US=300";;
  unmody2) ;;
esac
F="--chaos --max-timeslice=$TS $SK"
rm -rf $T; t0=$(date +%s)
out=$(timeout $TMO $H run --epoch=2026-01-01T00:00:00Z $F $ENVF --tmp=$T --summary-json=$J --sched-seed=$SEED -- $D/$BIN /tmp/w.db 300 1 0 5 4 0 3 1 2 2>$T.err); rc=$?
ov=$(grep -c "SKID_OVERSHOOT" $T.err 2>/dev/null)
st=$(python3 -c "import json;j=json.load(open('$J'));t=j['timeslice_stats'];print('syscalls=%d turns=%d slices=%d max_ns=%d'%(j['syscalls'],j['sched_turns'],t['count'],t['max_ns']))" 2>/dev/null)
echo "PILOT7 kind=$KIND ver=$VER seed=$SEED tag=$TAG rc=$rc secs=$(( $(date +%s)-t0 )) overshoot=$ov $st flags=[$F $ENVF yields=2] $(echo "$out" | head -1 | cut -c1-120) $( [ $rc -ne 0 ] && tail -c 200 $T.err | tr '\n' ' ')" >> $D/pilot7.results
rm -rf $T $T.err $J
