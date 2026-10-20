#!/usr/bin/env bash
# usage: taskset -c 0 setsid nohup timeout -k 30 2700 bash $R/bin/x1d.sh $R 1,2,3 <stamp> > $R/run.out 2>&1 < /dev/null &
# X1d: informational probe of P's pinned condlock binary under the pinned Hermit, 30 seeds at -r 125 and 30 at -r 500. Nothing is graded.
# Functions marked "x1c:" are byte-identical to x1c.sh; the others are new or adapted.
set -u
R=$1; CORES=$2; STAMP=$3; L=$R/logs; BIN=$R/bin
H=/dev/shm/x1d-glibc-$STAMP; HCORE=${CORES%%,*}
X1B=/path/to/x1b-glibc-20261017T1441Z   # EDIT: the work dir of x1b.sh; read only
CK=$R/ck; CLK=$CK/examples/glibc-condlock; CTL=$R/ctl
HERMIT=/path/to/hermit   # EDIT: the pinned hermit binary (sha256 must equal HERMIT_SHA)
HERMIT_SHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
CPATH=/usr/bin:/bin   # PATH of the plain runs and of the root Hermit wrapper: never changed
STEPPATH=$CPATH
T0=$(date +%s); NEWBY=$((T0 + 40 * 60))
RCAP_KB=102400; HCAP_KB=51200; FREE_MIN_KB=10485760; SHM_MIN_KB=204800
PIN_CLTAR=9eddf7b1d46ce88bc012b8eb76d0ba397b92ddacee8e538b5bb5bdada01f0014
PIN_CLS=5fbc06df3de905c7c67a745d1b8c93c62d886c182f98ac85f93c92433637d32a
PIN_X1B_CASES=0fd5a4a8f34c550d4e81d1d890ae1fa3fb40918e8f98bd02059f79a37e082eb1
PIN_X1B_STEPS=ea10be71b3e814b697d13db4068734cf7dfb805e06bb1b0b18b9093e0b485604
pin_of() { case "$1_$2" in   # KIND ARM (arm P only)
  LIBC_P) echo 07c76f357231b50801e8fb24e4556108f027717f28a32dd95d53f6458c63f471;;
  BIN_P) echo d920950e186ba1fbe476a804394b32fb6fe37125d33e74f8d54101b94a203153;;
  *) echo unknown-pin;;
esac; }
SM1_ARGS="-n 2 -r 1 -tick-ms 50 -q 10"
PX_BIN=condlock-P; PX_N=30; PX_CELLS="125 500"   # order of the cells; seeds: -r 500 from 319831, -r 125 from 319861
MOUNTS=/proc/mounts
V_PX=PX-NOT-RUN/PRIOR
INCH=
PX_STOP=0; PX_CONSEC=0; PX_RF=

# x1c: note
note() { printf '%s\t%s\n' "$(date +%s)" "$*" >> $R/events.tsv; }
# x1c: vr
vr() { printf 'K\t%s\t%s\t%s\n' "$1" "$2" "$3" >> $R/cases.tsv; note "K $1 $3"; }   # id, raw value, verdict
REFUSED=
# ok_to_start: as x1c (three lines adapted: 40 min line, 100 MB cap). Returns 0, or 1 with REFUSED = STOP|HALT|BUDGET|PAUSED|DISK|PROC.
ok_to_start() { local now; now=$(date +%s); REFUSED=
  [ -e $R/STOP ] && { REFUSED=STOP; note STOPPED; return 1; }
  [ -e $R/HALT ] && { REFUSED=HALT; note "HALTED: $(cat $R/HALT)"; return 1; }
  [ "$now" -ge $NEWBY ] && { REFUSED=BUDGET; note "BUDGET: no new step after 40 min"; return 1; }
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && { REFUSED=PAUSED; note "PAUSED: heavy job started"; return 1; }
  [ "$(du -sk $R | cut -f1)" -gt $RCAP_KB ] && { REFUSED=DISK; note "DISK: $R over 100 MB cap"; return 1; }
  [ -d $H ] && [ "$(du -sk $H 2> /dev/null | cut -f1)" -gt $HCAP_KB ] && { REFUSED=DISK; note "DISK: $H over 50 MB cap"; return 1; }
  [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -lt $FREE_MIN_KB ] && { REFUSED=DISK; note "DISK: less than 10 GB free"; return 1; }
  for _ in 1 2 3 4 5; do { procs_with_dir $R; procs_with_dir $H; } > $R/procs-now.txt; [ -s $R/procs-now.txt ] || break; sleep 1; done
  [ -s $R/procs-now.txt ] && { REFUSED=PROC; note "REFUSED: a process has the work dir in its arguments: $(head -n 1 $R/procs-now.txt | cut -c1-200)"; return 1; }
  return 0; }
# x1c: snapshot
snapshot() { { date -u +%FT%TZ; cat /proc/loadavg; ps -eo pid,ppid,pgid,sid,ni,psr,pcpu,etimes,stat,comm --sort=-pcpu | head -30; free -m; cat /proc/mdstat
    df -P -k "$HOME" /dev/shm; sha256sum /etc/ld.so.cache
    echo "cpu1 governor: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_governor 2>&1) cur_khz: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq 2>&1)"
    pgrep -a -x 'hermit|qemu-system-x86|cargo|rustc|make|cc1|cc1plus|gcc|ld|gdb'
    ls -la --time-style=full-iso /var/crash
    f() { awk '/^cpu[0-9]/{print $1,$5+$6,$2+$3+$4+$5+$6+$7+$8+$9}' /proc/stat; }; f > $R/s1.$1; sleep 5; f > $R/s2.$1
    paste $R/s1.$1 $R/s2.$1 | awk '{printf "%s %.1f%%\n",$1,100*(1-($5-$2)/($6-$3))}'; } > $R/snap-$1.txt 2>&1
  ls -1 /tmp | sort > $R/tmp-$1.txt; ls -la --time-style=full-iso /var/crash > $R/crash-$1.txt 2>&1; sha256sum /etc/ld.so.cache > $R/ldcache-$1.txt
  ls -1 /dev/shm | sort > $R/shm-$1.txt; ls -lan --time-style=full-iso /tmp /dev/shm > $R/own-$1.txt 2>&1   # own-: numeric owners, to list the root-owned entries that are left in place
  { du -sk $R; [ -d $H ] && du -sk $H; } > $R/du-$1.txt 2>&1; }
# x1c: leftovers
leftovers() { local p e; pgrep -s $SID > $R/sess.tmp   # the driver ($$) and the timeout that owns the session ($PPID) are excluded
  while read -r p; do [ "$p" = $$ ] || [ "$p" = $PPID ] || echo "session $p $(tr '\0' ' ' < /proc/$p/cmdline 2> /dev/null)"; done < $R/sess.tmp
  for p in /proc/[0-9]*; do e=$(readlink $p/exe 2> /dev/null); case $e in $R/*|$H/*) echo "exe-scan ${p#/proc/} $e";; esac; done
  ps -eo pid=,user=,sid=,args= | RP="$H/" awk 'index($0,ENVIRON["RP"]) { print "root-scan " $0 }'; }
# x1c: procs_with_dir
# every process of ANY user, in ANY session, whose arguments contain "$1/", except the driver's ancestor chain ANC and its own direct children
procs_with_dir() { ps -eo pid=,ppid=,user=,sid=,args= | RP="$1/" awk -v me="$$" -v anc="$ANC" 'index(anc, " " $1 " ") == 0 && $2!=me && index($0,ENVIRON["RP"])'; }
# x1c: step
step() { # id timeout cwd cmd... : one logged step, clean env, pinned cores, nice 10
  local id=$1 to=$2 cwd=$3 t0 rc; shift 3
  ok_to_start || return 99
  t0=$(date +%s.%N)
  ( ulimit -c 0; cd "$cwd" && env -i PATH=$STEPPATH HOME=$R LANG=C LC_ALL=C TMPDIR=$R/tmp nice -n 10 taskset -c $CORES timeout -k 30 "$to" "$@" ) > $L/$id.log 2>&1
  rc=$?
  printf '%s\t%s\t%s\t%s\n' "$id" "$rc" "$(awk -v a=$t0 -v b="$(date +%s.%N)" 'BEGIN{printf "%.1f",b-a}')" "$*" >> $R/steps.tsv; return $rc; }
# x1c: runone
runone() { local id=$1 tmo=$2; shift 2   # one plain run; prints the exit status; stdout and stderr to $L/<id>.out and .err
  ( ulimit -c 0; cd $R && env -i PATH=$CPATH HOME=$R LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$tmo" "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; echo $?; }
# x1c: sha_of
sha_of() { sha256sum "$1" 2> /dev/null | cut -d' ' -f1; }
# x1c: jnum
jnum() { sed -n "s/^ *\"$2\": *\([0-9][0-9]*\).*/\1/p" "$1" 2> /dev/null | head -n 1; }   # the first integer after "key": in a Hermit summary
# end_checks: adapted from x1c (the inputs differ)
end_checks() { local a bad=''
  INCH=
  if [ -e $R/condlock.tar ]; then [ "$(sha_of $R/condlock.tar)" = $PIN_CLTAR ] || INCH="$INCH+INPUT-CHANGED"; fi
  if [ -e $CLK/scripts/classify.sh ]; then [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || INCH="$INCH+INPUT-CHANGED"; fi
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || INCH="$INCH+HERMIT-CHANGED"
  if [ -d $X1B ]; then
    a=P
    [ "$(sha_of $X1B/$a/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC $a)" ] || bad=1
    [ "$(sha_of $X1B/ck/b-$a/condlock)" = "$(pin_of BIN $a)" ] || bad=1
    [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || bad=1
    [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || bad=1
  else bad=1; fi
  [ -z "$bad" ] || INCH="$INCH+X1B-CHANGED"
  (cd $BIN && sha256sum x1d.sh) > $R/end-helpers.txt 2>&1; }
# x1c: rootowned
# rootowned: everything under $H that is not owned by ubuntu (Hermit runs as root), listed and LEFT IN PLACE
rootowned() { find $H ! -user ubuntu -printf '%u %m %p\n' > $R/rootowned.txt 2>&1; vr Z-rootowned "$(grep -c . $R/rootowned.txt) lines (root-owned entries and permission errors, left in place)" INFO; }
# x1c: finish_up
finish_up() { end_checks; snapshot post; leftovers > $R/leftover.txt; rootowned; note "LEFTOVER=$(wc -l < $R/leftover.txt) DONE"; echo DONE >> $R/steps.tsv; }
# on_signal: as x1c with the X1D code
on_signal() { trap '' TERM INT HUP; echo SIGNAL > $R/HALT; note "SIGNAL: the driver was told to stop; HALT written"; finish_up
  printf 'OVERALL\tX1D-INCONCLUSIVE/SIGNAL\n' >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 6; }
gate_fail() { vr "G-$1" "gate failed" GATE-FAIL; note "GATE-FAIL $1"; echo "GATE-FAIL $1" >> $R/steps.tsv
  finish_up; printf 'OVERALL\tX1D-NOT-STARTED/%s\n' "$1" >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 5; }

# ---- G: gates on the reused X1b work dir (no repair), then the extraction of condlock.tar for classify.sh only
gates() {
  [ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/gate-sudo.log 2>&1 || gate_fail SUDO
  if grep ' /dev/shm ' $MOUNTS 2> /dev/null | grep -qw noexec; then gate_fail NOEXEC; fi
  { [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -ge $FREE_MIN_KB ] && [ "$(df -P -k /dev/shm | awk 'NR==2{print $4}')" -ge $SHM_MIN_KB ]; } || gate_fail FREE
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || gate_fail HERMIT-SHA
  [ -d $X1B ] || gate_fail REUSE-DIR
  [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || gate_fail REUSE-RECORD-CASES
  [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || gate_fail REUSE-RECORD-STEPS
  [ "$(sha_of $X1B/P/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC P)" ] || gate_fail REUSE-LIBC-P
  [ "$(sha_of $X1B/ck/b-P/condlock)" = "$(pin_of BIN P)" ] || gate_fail REUSE-BIN-P
  [ "$(sha_of $R/condlock.tar)" = $PIN_CLTAR ] || gate_fail CLTAR-SHA
  vr G-reuse "the X1b work dir is the one X1b recorded (P libc.a, P condlock, cases.tsv, steps.tsv)" OK; return 0; }
ck_extract() { local rc
  mkdir -p $CK
  step C-x-ck 300 $R tar -xf condlock.tar -C $CK; rc=$?; [ $rc = 0 ] || gate_fail CK-EXTRACT
  [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || gate_fail CLASSIFY-SHA
  if grep -qx 'SDK_COMMIT=a4131e49680fd80edf66d560a5cbf4be54de0cd7' $CLK/PINS && grep -qx 'SDK_TREE=f5d13d7773d6211e9ebb2283f774bc4b00cd6013' $CLK/PINS; then
    vr C-layout "$(find $CK -type f | wc -l | tr -d ' ') files" OK; return 0; fi
  gate_fail CK-LAYOUT; }
setup_copy() {
  mkdir -p $CTL $R/tmp
  cp $X1B/ck/b-P/condlock $CTL/x1b-condlock-P || gate_fail COPY-BIN-P
  [ "$(sha_of $CTL/x1b-condlock-P)" = "$(pin_of BIN P)" ] || gate_fail COPY-BIN-SHA-P
  return 0; }

# ---- H: the probe. hermit_run is byte-identical to x1c.
# x1c: hermit_run
hermit_run() { local id=$1 bin=$2 seed=$3 d=$H/runs/$1 t0 rc; shift 3
  mkdir -p $d/tmp $H/hout || { echo 99; return; }
  chmod 1777 $d/tmp $H/hout || { echo 99; return; }
  t0=$(date +%s%N)
  ( ulimit -c 0; cd $d && sudo -n timeout -k 10 120 env -i PATH=$CPATH HOME=/root LANG=C nice -n 10 taskset -c $HCORE $HERMIT run --epoch=2026-01-01T00:00:00Z --base-env=minimal \
      --chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot --sched-seed=$seed --tmp=$d/tmp --summary-json=$H/hout/$id.summary.json -- $H/$bin "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; rc=$?
  echo $(( ($(date +%s%N) - t0) / 1000000 )) > $L/$id.wallms; echo $rc; }
# px_class ID RC : like x1c's smoke_class, and the property flag (prop_false of the repo's classify.sh)
px_class() { local cl; cl=$(sh $CLK/scripts/classify.sh "$2" $L/$1.out $L/$1.err 2>&1 | head -n 1)
  PX_CLS=${cl%% *}; PX_CLS=${PX_CLS#class=}; PX_SUB=-; PX_PF=-
  if [[ $cl =~ subtype=([A-Za-z+-]+) ]]; then PX_SUB=${BASH_REMATCH[1]}; fi
  if [[ $cl =~ prop_false=([01]) ]]; then PX_PF=${BASH_REMATCH[1]}; fi; }
px_first() { case $1 in 500) echo 319831;; 125) echo 319861;; *) echo 0;; esac; }   # first seed of a cell
# px_cell R : PX_N serial Hermit runs of the probed binary at -r R; rows only, nothing graded; three consecutive failed runs end the whole probe
px_cell() { local r=$1 i seed id rc ticks dump fail
  for ((i = 0; i < PX_N; i++)); do
    seed=$(($(px_first $r) + i)); id=px-$r-$seed
    if [ $PX_STOP != 0 ]; then vr "H-$id" skipped PX-SKIPPED; continue; fi
    if ! ok_to_start; then PX_RF=$REFUSED; PX_STOP=refused; vr "H-$id" refused "PX-NOT-RUN/$PX_RF"; continue; fi
    rc=$(hermit_run $id $PX_BIN $seed -n 8 -r $r -tick-ms 100 -q 10)
    if [ "$rc" = 99 ]; then PX_RF=SETUP; PX_STOP=refused; vr "H-$id" "rc=99" PX-NOT-RUN/SETUP; continue; fi
    if [ "$rc" != 0 ] && [ ! -s $L/$id.out ] && grep -q -E '^sudo:' $L/$id.err; then PX_RF=SUDO; PX_STOP=refused; vr "H-$id" "rc=$rc" PX-NOT-RUN/SUDO; continue; fi
    px_class "$id" "$rc"
    ticks=$(sed -n 's/.* ticks= *\([0-9][0-9]*\).*/\1/p' $L/$id.out | head -n 1)
    dump=$(grep -m 1 '^condlock: ' $L/$id.err | cut -c1-300)
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$r" "$seed" "$rc" "$PX_CLS" "$PX_PF" "$PX_SUB" "$(cat $L/$id.wallms 2> /dev/null)" "$(jnum $H/hout/$id.summary.json virttime_elapsed)" "$(jnum $H/hout/$id.summary.json sched_turns)" "${ticks:-NA}" "${dump:-none}" >> $R/px.tsv
    vr "H-$id" "rc=$rc class=$PX_CLS prop_false=$PX_PF subtype=$PX_SUB wall_ms=$(cat $L/$id.wallms 2> /dev/null) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns) ticks=${ticks:-NA} dump=[${dump:-none}]" INFO
    fail=0; case $rc in 124|137|125|101) fail=1;; esac; [ "$PX_CLS" = HERMIT-FAIL ] && fail=1
    if [ $fail = 1 ]; then PX_CONSEC=$((PX_CONSEC + 1)); else PX_CONSEC=0; fi
    [ $PX_CONSEC -ge 3 ] && PX_STOP=failures
  done; }
px_summary() { local r=$1
  vr "PXSUM-$r" "$(awk -F'\t' -v r="$r" '$1==r{n++; if($5==1)k++; c[$4]++; w=$7+0; if(n==1||w<mn)mn=w; if(w>mx)mx=w; if(w>6000)o++; t+=$10}
    END{printf "runs=%d prop_false=%d wall_ms_min=%d wall_ms_max=%d wall_over_6000ms=%d ticks_sum=%d classes=[", n, k, mn, mx, o, t; for(x in c) printf "%s:%d,", x, c[x]; printf "]"}' $R/px.tsv)" INFO; }
px_phase() { local r rc
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || { V_PX=PX-NOT-RUN/PIN; vr PX "Hermit sha256 differs" $V_PX; return 0; }
  ok_to_start || { V_PX="PX-NOT-RUN/$REFUSED"; vr PX refused "$V_PX"; return 0; }
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/px-sudo.log 2>&1 || { V_PX=PX-NOT-RUN/SUDO; vr PX "sudo -n /bin/true failed" $V_PX; return 0; }
  mkdir -p $H/runs $H/hout || { V_PX=PX-NOT-RUN/NODIR; vr PX fail $V_PX; return 0; }
  chmod 1777 $H/hout
  cp $CTL/x1b-condlock-P $H/$PX_BIN 2> /dev/null
  cmp -s $CTL/x1b-condlock-P $H/$PX_BIN || { V_PX=PX-NOT-RUN/COPY; vr PX "$PX_BIN missing or the copy differs" $V_PX; return 0; }
  rc=$(runone px-exec 60 $H/$PX_BIN $SM1_ARGS)
  case $rc in 0|3|4) ;; *) V_PX=PX-NOT-RUN/NOEXEC; vr PX "rc=$rc" $V_PX; return 0;; esac
  : > $R/px.tsv; PX_STOP=0; PX_CONSEC=0; PX_RF=
  for r in $PX_CELLS; do px_cell $r; px_summary $r; done
  if [ $PX_STOP = failures ]; then V_PX=PX-STOPPED-FAILURES
  elif [ $PX_STOP = refused ]; then V_PX="PX-PARTIAL/$PX_RF"
  else V_PX=PX-RAN; fi
  vr PX "$(grep -c . $R/px.tsv) of $((PX_N * 2)) runs recorded" "$V_PX"; return 0; }

# ---- O: the overall code (mechanical; no count, class or time enters it)
overall() {
  case $V_PX in
    PX-RAN) echo X1D-RAN;;
    PX-STOPPED-FAILURES) echo X1D-HERMIT-STOPPED;;
    PX-PARTIAL/*) echo "X1D-INCONCLUSIVE/${V_PX#PX-PARTIAL/}";;
    PX-NOT-RUN/*) echo "X1D-INCONCLUSIVE/${V_PX#PX-NOT-RUN/}";;
    *) echo "X1D-UNKNOWN/$V_PX";;
  esac; }
side_effects() { local s=
  cmp -s $R/ldcache-before.txt $R/ldcache-post.txt || s="$s+LDCACHE-CHANGED"
  [ -s $R/leftover.txt ] && s="$s+LEFTOVER"
  cmp -s $R/tmp-before.txt $R/tmp-post.txt || s="$s+TMP-LISTING-DIFFERS"
  cmp -s $R/shm-before.txt $R/shm-post.txt || s="$s+SHM-LISTING-DIFFERS"
  cmp -s $R/crash-before.txt $R/crash-post.txt || s="$s+VAR-CRASH-DIFFERS"
  echo "$s$INCH"; }
overall_line() { printf 'OVERALL\t%s%s\tpx=%s\n' "$1" "$2" "$V_PX"; }

# ---- test seam: the Mac stub tests source this file with X1D_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${X1D_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "X1D_TEST is set in a real run: refused" >&2; exit 5; fi
  return 0
fi
SID=$(ps -o sid= -p $$ | tr -d ' '); echo "$SID $T0" > $R/sid.txt
# the driver's whole ancestor chain (this shell up to pid 1), once, as " pid pid ... " (GON/GOG guard.sh rule): the launch shell, nohup/timeout and anything above may have the work dir in their arguments
ANC=" "; p=$$
while [ -n "$p" ] && [ "$p" -gt 1 ]; do ANC="$ANC$p "; p=$(ps -o ppid= -p "$p" | tr -d ' '); done
unset p
trap on_signal TERM INT HUP
mkdir -p $R/tmp $H $L; : > $R/cases.tsv; snapshot pre
cp $R/tmp-pre.txt $R/tmp-before.txt; cp $R/crash-pre.txt $R/crash-before.txt; cp $R/ldcache-pre.txt $R/ldcache-before.txt; cp $R/shm-pre.txt $R/shm-before.txt

# ---- main
gates
ck_extract
setup_copy
px_phase
finish_up
overall_line "$(overall)" "$(side_effects)" >> $R/cases.tsv
echo DONE-OVERALL >> $R/steps.tsv
