#!/bin/bash
# Stage 1 + Stage 2 cost pilots, strictly sequential phases; a stop file halts at the next phase boundary.
W=/home/ubuntu/wal/pilot6_worker.sh; R=/opt/anthill-wal/pilot6.results
V2=3510200; V3=3510300
run() { [ -e /opt/anthill-wal/pilot6.stop ] && { echo STOPPED_AT_PHASE_BOUNDARY >> $R; exit 0; }; xargs -P 7 -L 1 $W; }
# 1a: reproducibility of the seed-3 hit
printf '%s\n' "jc100d $V2 3 r1" "jc100d $V2 3 r2" "jc100d $V2 3 r3" "jc100 $V2 3 s25" "jc100 $V3 3 s25" | run
# 1b-i: pre-registered control arm, 28 seeds on each version
{ for v in $V2 $V3; do for s in $(seq 1 28); do echo "jc100 $v $s a"; done; done; } | run
# 1b-ii: second control arm
{ for v in $V2 $V3; do for s in $(seq 1 28); do echo "jc550 $v $s b"; done; done; } | run
# verify seeds 3-5 on the unmodified build at skid 25
printf '%s\n' "ver $V2 3 v" "ver $V2 4 v" "ver $V2 5 v" | run
# Stage 2 cost pilots, rows alone
{ for s in 1 2 3 4 5 6 7; do echo "t250 $V2 $s c"; done; } | run
{ for s in 1 2 3 4 5 6 7; do echo "t100 $V2 $s c"; done; } | run
echo "PHASES_DONE" >> $R
