#!/bin/bash
# Stage 0d phases, strictly sequential; each phase uses 7 parallel workers at most and runs alone.
W=/home/ubuntu/wal/pilot4_worker.sh
run() { [ -e /opt/anthill-wal/pilot4.stop ] && { echo STOPPED_AT_PHASE_BOUNDARY >> /opt/anthill-wal/pilot4.results; exit 0; };  printf '%s\n' "$@" | xargs -P 7 -L 1 $W; }
run "vdef 1" "vdef 2" "v200 1" "v200 2" "v50 1" "v50 2"
run "b 1" "b 2" "b 3" "b 4" "b 5" "b 6" "b 7"
run "s200 1" "s200 2" "s200 3" "s200 4" "s200 5" "s200 6" "s200 7"
run "s50 1" "s50 2" "s50 3" "s50 4" "s50 5" "s50 6" "s50 7"
run "jc 1" "jc 2" "jc 3" "jc 4" "jc 5" "jc 6" "jc 7"
echo "PHASES_DONE" >> /opt/anthill-wal/pilot4.results
