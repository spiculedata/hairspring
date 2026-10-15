#!/bin/bash
# usage: native3.sh VER SECONDS DIR "ARGS" LOGFILE  -- 6 workers loop walwork7 natively, append a summary line to LOGFILE
VER=$1; SECS=$2; DIR=$3; ARGS=$4; LOG=$5; mkdir -p $DIR $(dirname $LOG)
end=$(( $(date +%s)+SECS ))
for w in 1 2 3 4 5 6; do
  ( runs=0; bad=0; while [ $(date +%s) -lt $end ]; do
      out=$(timeout 60 /home/ubuntu/wal/${BIN:-walwork7}-$VER $DIR/w$w.db $ARGS 2>&1); rc=$?; runs=$((runs+1))
      [ $rc -ne 0 ] && { bad=$((bad+1)); echo "w$w rc=$rc $out" | head -1 >> $DIR/viol.txt; }
    done; echo "$runs $bad" > $DIR/res.$w ) &
done; wait
cat $DIR/res.* | awk -v v=$VER -v b="${BIN:-walwork7}" -v a="$ARGS" -v d="${HS_DELAY_US:-0}" '{r+=$1;x+=$2} END{printf "%s ver=%s delay_us=%s args=[%s] runs=%d violations=%d\n", b, v, d, a, r, x}' | tee -a $LOG
head -2 $DIR/viol.txt 2>/dev/null | cut -c1-200 | tee -a $LOG; rm -rf $DIR
