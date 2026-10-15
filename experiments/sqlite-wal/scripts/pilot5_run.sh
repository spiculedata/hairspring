#!/bin/bash
# Stage 0e phases, strictly sequential, each phase alone; optional stop file halts at a phase boundary.
W=/home/ubuntu/wal/pilot5_worker.sh
run() { [ -e /opt/anthill-wal/pilot5.stop ] && { echo STOPPED_AT_PHASE_BOUNDARY >> /opt/anthill-wal/pilot5.results; exit 0; }; printf '%s\n' "$@" | xargs -P 7 -L 1 $W; }
run "jc2 1" "jc2 2" "jc2 3" "jc2 4" "jc2 5" "jc2 6" "jc2 7" "jdiag 1"
run "vdef 1" "vdef 2" "v50 1" "v50 2" "v25 1" "v25 2"
run "s25 1" "s25 2" "s25 3" "s25 4" "s25 5" "s25 6" "s25 7"
echo "PHASES_DONE" >> /opt/anthill-wal/pilot5.results
