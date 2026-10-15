#!/bin/bash
# usage: native2.sh VER SECONDS DIR "ARGS"   -- 6 parallel workers loop walwork2 and count violations
VER=$1; SECS=$2; DIR=$3; ARGS=$4; mkdir -p $DIR
end=$(( $(date +%s)+SECS ))
for w in 1 2 3 4 5 6; do
  ( runs=0; bad=0; while [ $(date +%s) -lt $end ]; do
      out=$(timeout 60 /home/ubuntu/wal/walwork2-$VER $DIR/w$w.db $ARGS 2>&1); rc=$?; runs=$((runs+1))
      [ $rc -ne 0 ] && { bad=$((bad+1)); echo "w$w rc=$rc $out" | head -2 >> $DIR/viol.txt; }
    done; echo "$runs $bad" > $DIR/res.$w ) &
done; wait
cat $DIR/res.* | awk -v v=$VER -v a="$ARGS" '{r+=$1;b+=$2} END{print "native2 " v " [" a "] runs=" r " violations=" b}'
