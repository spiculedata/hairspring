#!/bin/bash
# usage: sweep.sh VER "CFGS" FIRST_SEED LAST_SEED JOBS
VER=$1; CFGS=$2; A=$3; B=$4; J=${5:-7}
D=/opt/anthill-wal
echo 1 > /proc/sys/kernel/perf_event_paranoid
for cfg in $CFGS; do for s in $(seq $A $B); do echo "$VER $cfg $s"; done; done | xargs -P $J -L 1 /home/ubuntu/wal/sweep6_worker.sh
echo "SWEEPDONE ver=$VER cfgs=$CFGS seeds=$A-$B" >> $D/sweep6.results
