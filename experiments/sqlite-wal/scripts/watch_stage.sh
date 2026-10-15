#!/bin/bash
# Generic stage monitor. Env: RESULTS (path on box), TAG (line prefix, e.g. PILOT6), TOTAL (expected result lines),
# RUNNER (runner script name for pkill), STOPFILE, STALL_MIN (default 12), CAP_MIN (default 70).
# Completes on PHASES_DONE/STOPPED marker; stall alarm after STALL_MIN with no new result; at CAP_MIN touches STOPFILE
# (halts at the next phase boundary) and waits up to 12 more minutes; kills remaining jobs only on stall or lost grace.
SSH=(ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR "${BOX:?set BOX=user@host}")
: "${RESULTS:?}" "${TAG:?}" "${TOTAL:?}" "${RUNNER:?}" "${STOPFILE:?}"
STALL_MIN=${STALL_MIN:-12}; CAP_MIN=${CAP_MIN:-70}
start=$(date +%s); last=-1; stall=0; lost=0; n=0; fin=0; capped=0; capat=0
while true; do
  r=$("${SSH[@]}" "f=$RESULTS; echo \"\$(grep -c '^$TAG' \$f 2>/dev/null || echo 0) \$(grep -cE 'PHASES_DONE|STOPPED_AT_PHASE_BOUNDARY' \$f 2>/dev/null || echo 0)\"" 2>/dev/null)
  if [ -z "$r" ]; then lost=$((lost+1)); [ $lost -ge 10 ] && { echo "lost contact"; exit 1; }; sleep 30; continue; fi
  read n fin <<< "$r"; lost=0; el=$(( $(date +%s)-start ))
  echo "$(date +%T) $TAG results=$n/$TOTAL watch_elapsed=${el}s"
  [ "$fin" -ge 1 ] && { echo COMPLETE; break; }
  if [ "$n" = "$last" ]; then stall=$((stall+1)); else stall=0; fi
  [ $stall -ge $(( STALL_MIN*2 )) ] && { echo "STALLED: no new result for ${STALL_MIN} min"; break; }
  if [ $el -ge $(( CAP_MIN*60 )) ] && [ $capped -eq 0 ]; then echo "TIME CAP ${CAP_MIN} min: stopping at next phase boundary"; "${SSH[@]}" "sudo touch $STOPFILE"; capped=1; capat=$el; fi
  [ $capped -eq 1 ] && [ $(( el - capat )) -ge 720 ] && { echo "grace expired"; break; }
  last=$n; sleep 30
done
if [ "$fin" -lt 1 ]; then
  "${SSH[@]}" "sudo pkill -f '^bash /home/ubuntu/wal/$RUNNER'; sudo pkill -x xargs; sudo pkill -f '^/bin/bash /home/ubuntu/wal/pilot'; sudo pkill -x timeout; sudo pkill -x hermit; sleep 2; echo killed-remaining"
fi
"${SSH[@]}" "grep '^$TAG' $RESULTS | sort -k2,5 | cut -c1-320"
