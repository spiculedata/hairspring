#!/bin/bash
W=/home/ubuntu/wal/pilot7_worker.sh; R=/opt/anthill-wal/pilot7.results
run() { [ -e /opt/anthill-wal/pilot7.stop ] && { echo STOPPED_AT_PHASE_BOUNDARY >> $R; exit 0; }; xargs -P 7 -L 1 $W; }
{ for v in 3510200 3510300; do for s in 1 2 3 4 5 6 7; do echo "jc100y2 $v $s a"; done; done; } | run
echo "PHASES_DONE" >> $R
