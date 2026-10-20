#!/usr/bin/env bash
# usage: taskset -c 0 setsid nohup timeout -k 30 1800 bash $R/bin/x1c.sh $R 1,2,3 <stamp> > $R/run.out 2>&1 < /dev/null &
# X1c: link the 22 planted and violation-only condlock binaries against the X1b sysroots (no glibc is built), isolate, pin, rebuild once, smoke, and probe Hermit's exit statuses.
# The helper functions follow x1b.sh (G0 rev 8's g0.sh); section 15 of the spec says what changed.
set -u
R=$1; CORES=$2; STAMP=$3; L=$R/logs; BIN=$R/bin
H=/dev/shm/x1c-glibc-$STAMP; HCORE=${CORES%%,*}
X1B=/path/to/x1b-glibc-20261017T1441Z   # EDIT: the work dir of x1b.sh; read only: sysroots, SDK archives, counted binaries, record files
CK=$R/ck; CLK=$CK/examples/glibc-condlock; P1=$R/p1; P2=$R/p2; PIN=$R/pin; CTL=$R/ctl
HERMIT=/path/to/hermit   # EDIT: the pinned hermit binary (sha256 must equal HERMIT_SHA)
HERMIT_SHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
CPATH=/usr/bin:/bin   # PATH of the plain runs and of the root Hermit wrapper: never changed
STEPPATH=$CPATH
T0=$(date +%s); NEWBY=$((T0 + 25 * 60))
RCAP_KB=614400; HCAP_KB=51200; FREE_MIN_KB=10485760; SHM_MIN_KB=204800
# pins (spec section 3.1) and pre-registered settings (3.3, 6.7, 6.8); none is tuned after data
PIN_CLTAR=9eddf7b1d46ce88bc012b8eb76d0ba397b92ddacee8e538b5bb5bdada01f0014
PIN_CLS=5fbc06df3de905c7c67a745d1b8c93c62d886c182f98ac85f93c92433637d32a
PIN_X1B_CASES=0fd5a4a8f34c550d4e81d1d890ae1fa3fb40918e8f98bd02059f79a37e082eb1
PIN_X1B_STEPS=ea10be71b3e814b697d13db4068734cf7dfb805e06bb1b0b18b9093e0b485604
pin_of() { case "$1_$2" in   # KIND ARM
  LIBC_U) echo 7a3afc14df8b7c4d64278db94a559705469ba653347a0ba711d91ddbf909967e;;
  LIBC_C) echo bf3337587f8620e477ff30c6b7c948d5b8645b3ce812642aeba9c31b4a49868b;;
  LIBC_P) echo 07c76f357231b50801e8fb24e4556108f027717f28a32dd95d53f6458c63f471;;
  SDK_U) echo ab22ba0e23faa592e9a1ced8438b24fd58ecddc3ace3f340f6fd93407a7e5ac8;;
  SDK_C) echo 3b28d4032a296565a8bd3b6f7ffe6978a5c69949085c62e9a92b19f808617cec;;
  SDK_P) echo 258e15afa06875fa1fac2e0e02372e42e3c124653cf07da859864b5a370c482f;;
  BIN_U) echo 9a34111a27d44d28b580b09cc0d88fa27b4ed63ac3ff6d87c107aa6638e6169e;;
  BIN_C) echo 9999bfac20b6c369f38103d06adb644117bc4e6b5dad286ded3dcbd524f5b70f;;
  BIN_P) echo d920950e186ba1fbe476a804394b32fb6fe37125d33e74f8d54101b94a203153;;
  *) echo unknown-pin;;
esac; }
KS="1 2 5 20 50"
SM1_ARGS="-n 2 -r 1 -tick-ms 50 -q 10"; SM2_ARGS="-n 8 -r 60 -tick-ms 50 -q 10"; SM2_REPS=3
HX_ARGS="-n 8 -r 500 -tick-ms 100 -q 10"; HX_DPOS=condlock-drop-k1; HX_DSTALL=condlock-leak-k1
HX_SEEDS_DPOS="319731 319732 319733"; HX_SEEDS_DSTALL="319741 319742 319743"
PM_ARGS="-n 8 -r 60 -tick-ms 100 -q 10"; PM_FIRST=319801; PM_N=30; PM_BIN=condlock-P   # informational P-under-Hermit mini-cell (6.8b); X1b's pinned counted binary of arm P
MOUNTS=/proc/mounts
RE_OUT='^condlock libc=2\.40 hdr=2\.40 rounds= *[0-9]+ signals= *[0-9]+ blocked1= *[0-9]+ maxb= *[0-9]+ maxstag= *[0-9]+ ticks= *[0-9]+ spanned= *[0-9]+( applied=[01])?$'
V_BR=NOT-RUN/PRIOR; V_B1=NOT-RUN/PRIOR; V_B2=NOT-RUN/PRIOR; V_PIN=NOT-RUN/PRIOR; V_SMOKE=NOT-RUN/PRIOR; V_HX=HX-NOT-RUN/PRIOR; V_PM=PM-NOT-RUN/PRIOR
CTLDIFF=; INCH=

note() { printf '%s\t%s\n' "$(date +%s)" "$*" >> $R/events.tsv; }
vr() { printf 'K\t%s\t%s\t%s\n' "$1" "$2" "$3" >> $R/cases.tsv; note "K $1 $3"; }   # id, raw value, verdict
REFUSED=
# ok_to_start: returns 0, or returns 1 with REFUSED = STOP|HALT|BUDGET|PAUSED|DISK|PROC. A refusal is never a failure of the thing that was about to run.
ok_to_start() { local now; now=$(date +%s); REFUSED=
  [ -e $R/STOP ] && { REFUSED=STOP; note STOPPED; return 1; }
  [ -e $R/HALT ] && { REFUSED=HALT; note "HALTED: $(cat $R/HALT)"; return 1; }
  [ "$now" -ge $NEWBY ] && { REFUSED=BUDGET; note "BUDGET: no new step after 25 min"; return 1; }
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && { REFUSED=PAUSED; note "PAUSED: heavy job started"; return 1; }
  [ "$(du -sk $R | cut -f1)" -gt $RCAP_KB ] && { REFUSED=DISK; note "DISK: $R over 600 MB cap"; return 1; }
  [ -d $H ] && [ "$(du -sk $H 2> /dev/null | cut -f1)" -gt $HCAP_KB ] && { REFUSED=DISK; note "DISK: $H over 50 MB cap"; return 1; }
  [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -lt $FREE_MIN_KB ] && { REFUSED=DISK; note "DISK: less than 10 GB free"; return 1; }
  for _ in 1 2 3 4 5; do { procs_with_dir $R; procs_with_dir $H; } > $R/procs-now.txt; [ -s $R/procs-now.txt ] || break; sleep 1; done
  [ -s $R/procs-now.txt ] && { REFUSED=PROC; note "REFUSED: a process has the work dir in its arguments: $(head -n 1 $R/procs-now.txt | cut -c1-200)"; return 1; }
  return 0; }
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
leftovers() { local p e; pgrep -s $SID > $R/sess.tmp   # the driver ($$) and the timeout that owns the session ($PPID) are excluded
  while read -r p; do [ "$p" = $$ ] || [ "$p" = $PPID ] || echo "session $p $(tr '\0' ' ' < /proc/$p/cmdline 2> /dev/null)"; done < $R/sess.tmp
  for p in /proc/[0-9]*; do e=$(readlink $p/exe 2> /dev/null); case $e in $R/*|$H/*) echo "exe-scan ${p#/proc/} $e";; esac; done
  ps -eo pid=,user=,sid=,args= | RP="$H/" awk 'index($0,ENVIRON["RP"]) { print "root-scan " $0 }'; }
# every process of ANY user, in ANY session, whose arguments contain "$1/", except the driver's ancestor chain ANC and its own direct children
procs_with_dir() { ps -eo pid=,ppid=,user=,sid=,args= | RP="$1/" awk -v me="$$" -v anc="$ANC" 'index(anc, " " $1 " ") == 0 && $2!=me && index($0,ENVIRON["RP"])'; }
step() { # id timeout cwd cmd... : one logged step, clean env, pinned cores, nice 10
  local id=$1 to=$2 cwd=$3 t0 rc; shift 3
  ok_to_start || return 99
  t0=$(date +%s.%N)
  ( ulimit -c 0; cd "$cwd" && env -i PATH=$STEPPATH HOME=$R LANG=C LC_ALL=C TMPDIR=$R/tmp nice -n 10 taskset -c $CORES timeout -k 30 "$to" "$@" ) > $L/$id.log 2>&1
  rc=$?
  printf '%s\t%s\t%s\t%s\n' "$id" "$rc" "$(awk -v a=$t0 -v b="$(date +%s.%N)" 'BEGIN{printf "%.1f",b-a}')" "$*" >> $R/steps.tsv; return $rc; }
runone() { local id=$1 tmo=$2; shift 2   # one plain run; prints the exit status; stdout and stderr to $L/<id>.out and .err
  ( ulimit -c 0; cd $R && env -i PATH=$CPATH HOME=$R LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$tmo" "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; echo $?; }
sha_of() { sha256sum "$1" 2> /dev/null | cut -d' ' -f1; }
jnum() { sed -n "s/^ *\"$2\": *\([0-9][0-9]*\).*/\1/p" "$1" 2> /dev/null | head -n 1; }   # the first integer after "key": in a Hermit summary
sys_manifest() { local S=$X1B/$1/sysroot   # recorded, not gated: what the links consume besides libc.a
  echo "include_files=$(find $S/usr/include -type f | wc -l | tr -d ' ') include_sha256=$(cd $S/usr/include && find . -type f | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d' ' -f1) lib_top_sha256=$(cd $S/usr/lib && sha256sum ./*.a ./*.o 2> /dev/null | sha256sum | cut -d' ' -f1)"; }
# end_checks: the reused inputs, the pins and the Hermit pin are hashed again at the end
end_checks() { local a n s bad='' pinbad=''
  INCH=
  if [ -e $R/condlock.tar ]; then [ "$(sha_of $R/condlock.tar)" = $PIN_CLTAR ] || INCH="$INCH+INPUT-CHANGED"; fi
  if [ -e $CLK/scripts/classify.sh ]; then [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || INCH="$INCH+INPUT-CHANGED"; fi
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || INCH="$INCH+HERMIT-CHANGED"
  if [ -d $X1B ]; then
    for a in U C P; do
      [ "$(sha_of $X1B/$a/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC $a)" ] || bad=1
      [ "$(sha_of $X1B/ck/b-$a/sdk/libhairspring.a)" = "$(pin_of SDK $a)" ] || bad=1
      [ "$(sha_of $X1B/ck/b-$a/condlock)" = "$(pin_of BIN $a)" ] || bad=1; done
    [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || bad=1
    [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || bad=1
  else bad=1; fi
  [ -z "$bad" ] || INCH="$INCH+X1B-CHANGED"
  if [ -s $R/pin-table.tsv ]; then
    while IFS=$'\t' read -r n s _; do [ "$(sha_of $PIN/$n)" = "$s" ] || pinbad=1; done < $R/pin-table.tsv
    [ -z "$pinbad" ] || INCH="$INCH+PIN-CHANGED"
  fi
  (cd $BIN && sha256sum x1c.sh) > $R/end-helpers.txt 2>&1; }
# rootowned: everything under $H that is not owned by ubuntu (Hermit runs as root), listed and LEFT IN PLACE
rootowned() { find $H ! -user ubuntu -printf '%u %m %p\n' > $R/rootowned.txt 2>&1; vr Z-rootowned "$(grep -c . $R/rootowned.txt) lines (root-owned entries and permission errors, left in place)" INFO; }
finish_up() { end_checks; snapshot post; leftovers > $R/leftover.txt; rootowned; note "LEFTOVER=$(wc -l < $R/leftover.txt) DONE"; echo DONE >> $R/steps.tsv; }
# on_signal: TERM, INT or HUP (the 1800 s cap, a Mac-side kill of the ssh session is not one): HALT, the post snapshot and the leftover scan, then exit 6
on_signal() { trap '' TERM INT HUP; echo SIGNAL > $R/HALT; note "SIGNAL: the driver was told to stop; HALT written"; finish_up
  printf 'OVERALL\tX1C-INCONCLUSIVE/SIGNAL\n' >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 6; }
gate_fail() { vr "G-$1" "gate failed" GATE-FAIL; note "GATE-FAIL $1"; echo "GATE-FAIL $1" >> $R/steps.tsv
  finish_up; printf 'OVERALL\tX1C-NOT-STARTED/%s\n' "$1" >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 5; }

# ---- names, per-name specification, compiler line
all_names() { local p k; for p in drop leak barrier holdg; do for k in $KS; do echo "condlock-$p-k$k"; done; done; echo condlock-novio-U; echo condlock-novio-P; }
armdef() { case $1 in C) echo CONDLOCK_ARM_C;; *) echo CONDLOCK_ARM_U;; esac; }   # P has the U layout (X1b)
# spec_of NAME : SP_ARM (sysroot arm), SP_KIND (sdk|novio), SP_TARGET (under BUILD), SP_PLANT, SP_K, SP_TOK (the define the name stands for); returns 1 for an unknown name
spec_of() { local n=$1 p
  SP_PLANT=-; SP_K=-
  case $n in
    condlock-novio-U) SP_ARM=U; SP_KIND=novio; SP_TARGET=condlock-novio; SP_TOK=-DCONDLOCK_NO_SDK; return 0;;
    condlock-novio-P) SP_ARM=P; SP_KIND=novio; SP_TARGET=condlock-novio; SP_TOK=-DCONDLOCK_NO_SDK; return 0;;
    condlock-drop-k*|condlock-leak-k*|condlock-barrier-k*|condlock-holdg-k*)
      p=${n#condlock-}; SP_PLANT=${p%%-k*}; SP_K=${p##*-k}; SP_ARM=C; SP_KIND=sdk; SP_TARGET=planted/$n
      case $SP_PLANT in
        drop) SP_TOK=-DCONDLOCK_DROP_SIGNAL=$SP_K;;
        leak) SP_TOK=-DCONDLOCK_PLANT_LEAK=$SP_K;;
        barrier) SP_TOK=-DCONDLOCK_PLANT_BARRIER=$SP_K;;
        holdg) SP_TOK=-DCONDLOCK_PLANT_HOLDG=$SP_K;;
      esac; return 0;;
  esac; return 1; }
cc_of() { local S=$X1B/$1/sysroot; echo "gcc --sysroot=$S -B$S/usr/lib/ -static -L$S/usr/lib -Wl,-t,-t"; }   # X1b's CC line, byte for byte
inc_viol() { # log sysroot : absolute include paths in a gcc -M log that are neither in the sysroot, nor gcc's own, nor the condlock tree
  tr -s ' \\' '\n' < $L/$1.log | grep '^/' | grep -v -e "^$2/" -e "^$(realpath "$2")/" -e '^/usr/lib/gcc/x86_64-linux-gnu/13/' -e "^$CK/" | wc -l | tr -d ' '; }
branch_of() { # preprocessed cl_condraw.c : which layout branch is live (the silent ARM_NONE fallback shows as 0 and 0)
  echo "arm_u=$(grep -c 'raw.g_refs\[0\] = c->__data.__g_refs\[0\]' $1) arm_c=$(grep -c 'raw.arm = CL_ARM_C;' $1) value64=$(grep -c 'raw.wseq = c->__data.__wseq.__value64' $1)"; }
iso_trace() { # sysroot log tag : prints the number of violations; raw offenders go to $L/iso-<tag>.txt
  local S=$1 log=$2 t=$3 pat
  pat='(crt1|crti|crtn|Scrt1|rcrt1|gcrt1)\.o|lib(c|m|pthread|dl|rt|util|resolv|anl|mvec|c_nonshared)\.(a|so)|ld-linux'
  grep -v '^attempt to open .* failed' $log > $L/iso-$t.tr
  { grep -E "$pat" $L/iso-$t.tr | grep -vF "$S/"; grep -E '/(usr/)?lib(64)?/x86_64-linux-gnu/' $L/iso-$t.tr; } > $L/iso-$t.txt
  grep -c . $L/iso-$t.txt; }
libc_hits() { grep -cF -e "$2/usr/lib/libc.a(" -e "($2/usr/lib/libc.a)" "$1"; }   # both trace forms: path(member) and (path)member

# ---- G: gates on the reused X1b work dir (no repair, no rebuild), then the extraction of condlock.tar
gates() { local a f
  [ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/gate-sudo.log 2>&1 || gate_fail SUDO
  if grep ' /dev/shm ' $MOUNTS 2> /dev/null | grep -qw noexec; then gate_fail NOEXEC; fi
  { [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -ge $FREE_MIN_KB ] && [ "$(df -P -k /dev/shm | awk 'NR==2{print $4}')" -ge $SHM_MIN_KB ]; } || gate_fail FREE
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || gate_fail HERMIT-SHA
  [ "$(gcc -dumpfullversion 2> /dev/null)" = 13.3.0 ] || gate_fail GCC-VERSION
  [ "$(all_names | LC_ALL=C sort -u | wc -l | tr -d ' ')" = 22 ] || gate_fail NAMES-22
  [ -d $X1B ] || gate_fail REUSE-DIR
  [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || gate_fail REUSE-RECORD-CASES
  [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || gate_fail REUSE-RECORD-STEPS
  for a in U C P; do
    for f in usr/lib/libc.a usr/lib/crt1.o usr/lib/crti.o usr/lib/crtn.o usr/include/gnu/libc-version.h; do [ -f $X1B/$a/sysroot/$f ] || gate_fail REUSE-SYSROOT-$a; done
    [ "$(sha_of $X1B/$a/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC $a)" ] || gate_fail REUSE-LIBC-$a
    [ "$(sha_of $X1B/ck/b-$a/sdk/libhairspring.a)" = "$(pin_of SDK $a)" ] || gate_fail REUSE-SDK-$a
    [ "$(sha_of $X1B/ck/b-$a/condlock)" = "$(pin_of BIN $a)" ] || gate_fail REUSE-BIN-$a
    vr "G-manifest-$a" "$(sys_manifest $a)" INFO
  done
  [ "$(sha_of $R/condlock.tar)" = $PIN_CLTAR ] || gate_fail CLTAR-SHA
  vr G-reuse "the X1b work dir is the one X1b recorded (libc.a, SDK archives, counted binaries, cases.tsv, steps.tsv)" OK; return 0; }
ck_extract() { local rc
  mkdir -p $CK
  step C-x-ck 300 $R tar -xf condlock.tar -C $CK; rc=$?; [ $rc = 0 ] || gate_fail CK-EXTRACT
  [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || gate_fail CLASSIFY-SHA
  if grep -qx 'SDK_COMMIT=a4131e49680fd80edf66d560a5cbf4be54de0cd7' $CLK/PINS && grep -qx 'SDK_TREE=f5d13d7773d6211e9ebb2283f774bc4b00cd6013' $CLK/PINS \
    && [ -f $CLK/src/cl_condraw.c ] && [ -f $CLK/src/cl_plant.h ] && [ -f $CLK/src/main.c ] && [ -f $CLK/Makefile ] && [ -f $CK/sdks/c/Makefile ]; then
    vr C-layout "$(find $CK -type f | wc -l | tr -d ' ') files" OK; return 0; fi
  gate_fail CK-LAYOUT; }
setup_copies() { local a
  mkdir -p $P1 $P2 $PIN $CTL $R/tmp
  for a in U C P; do mkdir -p $CK/b-$a/sdk
    cp $X1B/ck/b-$a/sdk/libhairspring.a $CK/b-$a/sdk/libhairspring.a || gate_fail COPY-SDK-$a
    [ "$(sha_of $CK/b-$a/sdk/libhairspring.a)" = "$(pin_of SDK $a)" ] || gate_fail COPY-SDK-SHA-$a
    cp $X1B/ck/b-$a/condlock $CTL/x1b-condlock-$a || gate_fail COPY-BIN-$a
    [ "$(sha_of $CTL/x1b-condlock-$a)" = "$(pin_of BIN $a)" ] || gate_fail COPY-BIN-SHA-$a
  done; return 0; }

# ---- BR: arm branch and include isolation, once per sysroot arm (X1b ck_arm step 3)
branch_gate() { local a S br nv want rc
  local -a fl
  for a in C U P; do
    S=$X1B/$a/sysroot; fl=("--sysroot=$S" "-B$S/usr/lib/" -std=gnu11 -O2 -g -Wall -Wextra -Werror -pthread -Isrc "-D$(armdef $a)")
    step br-E-$a 120 $CLK gcc "${fl[@]}" -E src/cl_condraw.c; rc=$?
    if [ $rc = 99 ]; then V_BR="NOT-RUN/$REFUSED"; return 1; fi
    step br-M-$a 120 $CLK gcc "${fl[@]}" -M src/cl_condraw.c; [ $? = 99 ] && { V_BR="NOT-RUN/$REFUSED"; return 1; }
    nv=$(inc_viol br-M-$a "$S"); br=$(branch_of $L/br-E-$a.log)
    case $a in C) want="arm_u=0 arm_c=1 value64=1";; *) want="arm_u=1 arm_c=0 value64=1";; esac
    vr BRANCH-$a "$br include_violations=$nv" INFO
    if [ $rc != 0 ] || [ "$nv" != 0 ] || [ "$br" != "$want" ]; then V_BR="FAIL-BRANCH:$a"; return 1; fi
  done; V_BR=BR-OK; vr BRANCH "arm branch active and every include from the sysroot, arms C U P" BR-OK; return 0; }

# ---- B: build the 22 binaries (link only)
# def_check NAME : the -DCONDLOCK_* tokens of the make -n command line are exactly {arm define, the name's define}. 0 ok, 1 bad, 99 refused.
def_check() { local n=$1 B rc got want
  spec_of "$n" || return 1
  B=$CK/b-$SP_ARM
  step "def-$n" 60 $CLK env "CFLAGS=-D$(armdef $SP_ARM)" make -n "$B/$SP_TARGET" "CC=$(cc_of $SP_ARM)" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && return 99
  [ $rc = 0 ] || return 1
  got=$(tr -s ' \\' '\n' < "$L/def-$n.log" | grep -E '^-DCONDLOCK_' | LC_ALL=C sort | tr '\n' ' ')
  want=$(printf '%s\n%s\n' "-D$(armdef $SP_ARM)" "$SP_TOK" | LC_ALL=C sort | tr '\n' ' ')
  vr "DEF-$n" "defines=[$got]" "$([ "$got" = "$want" ] && echo DEF-OK || echo "DEF-BAD want=[$want]")"
  [ "$got" = "$want" ]; }
# build_one PASS NAME OUTDIR : one make; the product is moved to OUTDIR/NAME. 0 ok, 1 failed, 99 refused.
build_one() { local ps=$1 n=$2 out=$3 B rc
  spec_of "$n" || return 1
  B=$CK/b-$SP_ARM
  step "b$ps-$n" 120 $CLK env "CFLAGS=-D$(armdef $SP_ARM)" make -s "$B/$SP_TARGET" "CC=$(cc_of $SP_ARM)" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && return 99
  { [ $rc = 0 ] && [ -x "$B/$SP_TARGET" ]; } || return 1
  mv "$B/$SP_TARGET" "$out/$n"; }
# bin_iso NAME : X1b's CISO gates for one pass 1 binary (6.4 item 3)
bin_iso() { local n=$1 b=$P1/$1 lg=$L/b1-$1.log S B v c sn v2=0 v3 why=
  spec_of "$n"; S=$X1B/$SP_ARM/sysroot; B=$CK/b-$SP_ARM
  v=$(iso_trace "$S" "$lg" "bi-$n"); [ "$v" = 0 ] || why="$why trace-violations=$v"
  c=$(libc_hits "$lg" "$S"); [ "$c" != 0 ] || why="$why no-libc-member-line"
  if [ "$SP_KIND" = sdk ]; then
    sn=$(grep -cF "$B/sdk/libhairspring.a" "$lg"); [ "$sn" != 0 ] || why="$why no-sdk-member-line"
    grep -F "$B/sdk/libhairspring.a" "$lg" > "$L/bi-$n.sdklines"   # the SDK archive is not a glibc component: its own lines pass the glibc rule
    v2=$(iso_trace "$S" "$L/bi-$n.sdklines" "sdkctl-$n"); [ "$v2" = 0 ] || why="$why sdk-counted-as-glibc"
  else
    sn=$(grep -c 'libhairspring' "$lg"); [ "$sn" = 0 ] || why="$why sdk-in-violation-only-link"
  fi
  printf '%s\n' '(/usr/lib/x86_64-linux-gnu/libc.a)printf.o' > "$L/bi-$n.negctl"
  v3=$(iso_trace "$S" "$L/bi-$n.negctl" "negctl-$n"); [ "$v3" != 0 ] || why="$why rule-insensitive"
  readelf -lW $b > $L/bi-$n.lw 2>&1; readelf -dW $b > $L/bi-$n.dw 2>&1; readelf -hW $b > $L/bi-$n.hw 2>&1; timeout 10 ldd $b > $L/bi-$n.ldd 2>&1
  [ "$(grep -c INTERP $L/bi-$n.lw)" = 0 ] || why="$why interp"
  grep -q 'There is no dynamic section' $L/bi-$n.dw || why="$why dynamic-section"
  grep -q 'not a dynamic executable' $L/bi-$n.ldd || why="$why ldd"
  grep -q 'Advanced Micro Devices X86-64' $L/bi-$n.hw || why="$why machine"
  strings -a $b > $L/bi-$n.strings; [ "$(grep -c 'Ubuntu GLIBC' $L/bi-$n.strings)" = 0 ] || why="$why ubuntu-glibc-string"
  vr "ISO-$n" "violations=$v libc_member_lines=$c sdk_lines=$sn sdk_control=$v2 negative_control=$v3 ${why:-none}" "$([ -z "$why" ] && echo ISO-OK || echo "NOT-ISO:${why# }")"
  [ -z "$why" ]; }
pass1() { local n rc h
  : > $R/pass1.sha
  for n in $(all_names); do
    def_check "$n"; rc=$?
    if [ $rc = 99 ]; then V_B1="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then V_B1="FAIL-DEF:$n"; return 1; fi
    build_one 1 "$n" "$P1"; rc=$?
    if [ $rc = 99 ]; then V_B1="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then vr "B1-$n" "make rc!=0 or no product" BUILD-FAIL; V_B1="FAIL-BUILD:$n"; return 1; fi
    bin_iso "$n" || { V_B1="FAIL-ISO:$n"; return 1; }
    h=$(sha_of "$P1/$n"); echo "$h  $n" >> $R/pass1.sha; vr "B1-$n" "$h" BUILT
  done
  if [ "$(awk '{print $1}' $R/pass1.sha | LC_ALL=C sort | uniq -d | grep -c .)" != 0 ]; then vr B1-uniq "two binaries share a sha256" DUPLICATE; V_B1="FAIL-UNIQ:duplicate-sha256"; return 1; fi
  V_B1=B1-OK; vr B1 "22 binaries built, isolated, distinct" B1-OK; return 0; }
# cmp_bins A B : "identical", or what differs: bytes, sizes, and whether the files are equal once debug info and the build-id are stripped
cmp_bins() { local a=$1 b=$2 n s=differ t=$R/tmp
  if cmp -s "$a" "$b"; then echo identical; return 0; fi
  n=$(cmp -l "$a" "$b" 2> /dev/null | wc -l | tr -d ' ')
  rm -f "$t/cb.a" "$t/cb.b"
  strip --strip-debug -R .note.gnu.build-id -o "$t/cb.a" "$a" 2> /dev/null; strip --strip-debug -R .note.gnu.build-id -o "$t/cb.b" "$b" 2> /dev/null
  if [ -s "$t/cb.a" ] && cmp -s "$t/cb.a" "$t/cb.b"; then s=equal; fi
  echo "differ bytes=$n sizes=$(wc -c < "$a" | tr -d ' ')/$(wc -c < "$b" | tr -d ' ') debug-and-buildid-stripped=$s"; }
pass2() { local n rc h1 h2 nd=0
  : > $R/pass2.sha
  for n in $(all_names); do
    build_one 2 "$n" "$P2"; rc=$?
    if [ $rc = 99 ]; then V_B2="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then vr "B2-$n" "make rc!=0 or no product" BUILD-FAIL; V_B2="FAIL-BUILD2:$n"; return 1; fi
    h1=$(sha_of "$P1/$n"); h2=$(sha_of "$P2/$n"); echo "$h2  $n" >> $R/pass2.sha
    if [ "$h1" = "$h2" ]; then vr "B2-$n" "$h2" IDENTICAL
    else nd=$((nd + 1)); vr "B2-$n" "pass1=$h1 pass2=$h2 $(cmp_bins "$P1/$n" "$P2/$n")" DIFFERS; fi
  done
  if [ $nd = 0 ]; then V_B2=REPRO; else V_B2="NONREPRO-$nd"; fi
  vr B2 "$nd of 22 differ between pass 1 and pass 2" "$V_B2"; return 0; }
# ctl_phase: INFORMATIONAL: the three counted condlock binaries rebuilt here, compared with X1b's pinned files (path dependence anchor). Never graded.
ctl_phase() { local a B rc r
  for a in U C P; do B=$CK/b-$a
    step ctl-$a 300 $CLK env "CFLAGS=-D$(armdef $a)" make -s condlock "CC=$(cc_of $a)" "BUILD=$B"; rc=$?
    if [ $rc = 99 ]; then vr CTL-$a refused "INFO-NOT-RUN/$REFUSED"; continue; fi
    if [ $rc != 0 ] || [ ! -x "$B/condlock" ]; then vr CTL-$a "rc=$rc" INFO-BUILD-FAILED; CTLDIFF="${CTLDIFF}$a"; continue; fi
    mv "$B/condlock" "$CTL/condlock-$a"
    r=$(cmp_bins "$CTL/condlock-$a" "$CTL/x1b-condlock-$a")
    [ "$r" = identical ] || CTLDIFF="${CTLDIFF}$a"
    vr "CTL-$a" "x1c=$(sha_of "$CTL/condlock-$a") x1b=$(pin_of BIN $a) $r" INFO
  done; return 0; }
pin_phase() { local n
  : > $R/pin-table.tsv
  for n in $(all_names); do
    spec_of "$n"
    if ! { cp "$P1/$n" "$PIN/$n" && chmod 555 "$PIN/$n" && cmp -s "$P1/$n" "$PIN/$n"; }; then V_PIN="FAIL-PIN:$n"; vr PIN "copy of $n failed" "$V_PIN"; return 1; fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$n" "$(sha_of "$PIN/$n")" "$(wc -c < "$PIN/$n" | tr -d ' ')" "$SP_ARM" "$SP_TOK" >> $R/pin-table.tsv
  done
  { for n in U C P; do printf 'libc.a-%s\t%s\n' "$n" "$(pin_of LIBC $n)"; done
    for n in U C P; do printf 'libhairspring.a-%s\t%s\n' "$n" "$(pin_of SDK $n)"; done
    for n in U C P; do printf 'x1b-condlock-%s\t%s\n' "$n" "$(pin_of BIN $n)"; done
    printf 'condlock.tar\t%s\nclassify.sh\t%s\nhermit\t%s\ngcc\t%s\n' "$PIN_CLTAR" "$PIN_CLS" "$HERMIT_SHA" "$(gcc -dumpfullversion 2> /dev/null)"; } > $R/pin-inputs.tsv
  V_PIN=PINNED; vr PIN "$(grep -c . $R/pin-table.tsv) rows; table sha256 $(sha_of $R/pin-table.tsv)" PINNED; return 0; }

# ---- S: plain smoke on the pinned pass 1 files. Classes come from the repo's classify.sh (pinned by sha256).
line_ok() { # file planted(0|1) : exactly one non-empty line, in the report format, with ` applied=` iff planted
  local f=$1 pl=$2 first
  [ "$(grep -c . "$f")" = 1 ] || return 1
  first=$(grep -m 1 . "$f")
  [[ $first =~ $RE_OUT ]] || return 1
  if [ "$pl" = 1 ]; then [[ $first == *" applied="* ]]; else [[ $first != *" applied="* ]]; fi; }
smoke_class() { # id rc : sets SM_CLS SM_AP SM_SUB from classify.sh
  local cl; cl=$(sh $CLK/scripts/classify.sh "$2" $L/$1.out $L/$1.err 2>&1 | head -n 1)
  SM_CLS=${cl%% *}; SM_CLS=${SM_CLS#class=}; SM_SUB=-; SM_AP=-
  if [[ $cl =~ subtype=([A-Za-z+-]+) ]]; then SM_SUB=${BASH_REMATCH[1]}; fi
  if [[ $cl =~ applied=([01-]) ]]; then SM_AP=${BASH_REMATCH[1]}; fi; }
# t2_expect NAME RC CLASS APPLIED : prints OK or the reason (the pre-registered table of 6.7)
t2_expect() { local n=$1 rc=$2 cls=$3 ap=$4
  spec_of "$n" || { echo unknown-name; return; }
  if [ "$SP_KIND" = novio ]; then
    case $rc in 0|3|4) ;; *) echo "rc=$rc"; return;; esac
    case $cls in NOHIT|HIT|STALL) echo OK;; *) echo "class=$cls";; esac; return; fi
  case $SP_PLANT in
    drop)
      if [ "$ap" = 1 ]; then { [ "$rc" = 3 ] && [ "$cls" = HIT ]; } && echo OK || echo "applied=1 rc=$rc class=$cls"
      elif [ "$ap" = 0 ]; then { [ "$rc" = 0 ] && [ "$cls" = NOHIT ]; } && echo OK || echo "applied=0 rc=$rc class=$cls"
      else echo "applied=$ap"; fi;;
    leak|barrier|holdg) { [ "$ap" = 1 ] && [ "$rc" = 4 ] && [ "$cls" = STALL ]; } && echo OK || echo "applied=$ap rc=$rc class=$cls";;
    *) echo unknown-plant;;
  esac; }
smoke1() { local n=$1 id=sm1-$1 rc pl why=
  spec_of "$n"; pl=0; [ "$SP_KIND" = sdk ] && pl=1
  rc=$(runone "$id" 60 "$P1/$n" $SM1_ARGS)
  case $rc in 0|3|4) ;; *) why="$why rc=$rc";; esac
  line_ok "$L/$id.out" $pl || why="$why report-line"
  vr "SM1-$n" "rc=$rc" "$([ -z "$why" ] && echo OK || echo "BAD:${why# }")"
  [ -z "$why" ]; }
smoke2() { local n=$1 rep=$2 id=sm2-$1-$2 rc pl why='' r
  spec_of "$n"; pl=0; [ "$SP_KIND" = sdk ] && pl=1
  rc=$(runone "$id" 60 "$P1/$n" $SM2_ARGS)
  smoke_class "$id" "$rc"
  line_ok "$L/$id.out" $pl || why="$why report-line"
  r=$(t2_expect "$n" "$rc" "$SM_CLS" "$SM_AP"); [ "$r" = OK ] || why="$why $r"
  vr "SM2-$n-$rep" "rc=$rc class=$SM_CLS applied=$SM_AP subtype=$SM_SUB" "$([ -z "$why" ] && echo OK || echo "BAD:${why# }")"
  [ -z "$why" ]; }
smoke_phase() { local n rep bad nbad=0 rf=
  for n in $(all_names); do
    ok_to_start || { rf=$REFUSED; break; }
    bad=0; smoke1 "$n" || bad=1
    for rep in $(seq $SM2_REPS); do smoke2 "$n" "$rep" || bad=1; done
    if [ $bad = 0 ]; then vr "SM-$n" "tier 1 and $SM2_REPS tier 2 runs as expected" SMOKE-OK; else nbad=$((nbad + 1)); vr "SM-$n" "a run was not as expected" SMOKE-BAD; fi
  done
  if [ -n "$rf" ]; then V_SMOKE="NOT-RUN/$rf"; vr SMOKE "refused before $n" "$V_SMOKE"; return 1; fi
  if [ $nbad = 0 ]; then V_SMOKE=SMOKE-OK; else V_SMOKE="FAIL-SMOKE:$nbad"; fi
  vr SMOKE "$nbad of 22 binaries had a run that was not as expected" "$V_SMOKE"; [ $nbad = 0 ]; }

# ---- H: the exit-status probe under the pinned Hermit (root via sudo -n; every path Hermit writes is in a 1777 directory). Prints the rc (99 = setup problem).
hermit_run() { local id=$1 bin=$2 seed=$3 d=$H/runs/$1 t0 rc; shift 3
  mkdir -p $d/tmp $H/hout || { echo 99; return; }
  chmod 1777 $d/tmp $H/hout || { echo 99; return; }
  t0=$(date +%s%N)
  ( ulimit -c 0; cd $d && sudo -n timeout -k 10 120 env -i PATH=$CPATH HOME=/root LANG=C nice -n 10 taskset -c $HCORE $HERMIT run --epoch=2026-01-01T00:00:00Z --base-env=minimal \
      --chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot --sched-seed=$seed --tmp=$d/tmp --summary-json=$H/hout/$id.summary.json -- $H/$bin "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; rc=$?
  echo $(( ($(date +%s%N) - t0) / 1000000 )) > $L/$id.wallms; echo $rc; }
# hx_eval RC CLASS APPLIED WANT_RC : HX-OK | HX-UNAPPLIED | HX-HERMITFAIL | HX-OTHER (6.8); nothing is interpreted
hx_eval() { local rc=$1 cls=$2 ap=$3 want=$4 wantcls
  case $want in 3) wantcls=HIT;; *) wantcls=STALL;; esac
  if [ "$cls" = HERMIT-FAIL ] || [ "$rc" = 125 ] || [ "$rc" = 101 ]; then echo HX-HERMITFAIL
  elif [ "$rc" = "$want" ] && [ "$cls" = "$wantcls" ] && [ "$ap" = 1 ]; then echo HX-OK
  elif [ "$rc" = 0 ] && [ "$cls" = NOHIT ] && [ "$ap" = 0 ]; then echo HX-UNAPPLIED
  else echo HX-OTHER; fi; }
hermit_phase() { local b seeds want seed id rc w v stop=0 nok_d=0 nok_s=0 nun=0 noth=0 nhf=0 rf=
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || { V_HX=HX-NOT-RUN/PIN; vr HX-pin "Hermit sha256 differs" $V_HX; return 0; }
  ok_to_start || { V_HX="HX-NOT-RUN/$REFUSED"; vr HX-start refused "$V_HX"; return 0; }
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/hx-sudo.log 2>&1 || { V_HX=HX-NOT-RUN/SUDO; vr HX-sudo "sudo -n /bin/true failed" $V_HX; return 0; }
  mkdir -p $H/runs $H/hout || { V_HX=HX-NOT-RUN/NODIR; vr HX-dir fail $V_HX; return 0; }
  chmod 1777 $H/hout
  for b in $HX_DPOS $HX_DSTALL; do
    cp $PIN/$b $H/$b 2> /dev/null
    cmp -s $PIN/$b $H/$b || { V_HX=HX-NOT-RUN/COPY; vr HX-copy "$b missing or the copy differs" $V_HX; return 0; }
  done
  rc=$(runone hx-exec 60 $H/$HX_DSTALL $SM1_ARGS)
  case $rc in 0|3|4) ;; *) V_HX=HX-NOT-RUN/NOEXEC; vr HX-exec "rc=$rc" $V_HX; return 0;; esac
  for b in $HX_DPOS $HX_DSTALL; do
    if [ "$b" = $HX_DPOS ]; then want=3; seeds=$HX_SEEDS_DPOS; else want=4; seeds=$HX_SEEDS_DSTALL; fi
    for seed in $seeds; do
      id=hx-$b-$seed
      if [ $stop = 1 ]; then vr "H-$id" "not started after a run that was not as expected or a refusal" HX-SKIPPED; continue; fi
      if ! ok_to_start; then rf=$REFUSED; stop=1; vr "H-$id" refused "NOT-RUN/$rf"; continue; fi
      rc=$(hermit_run $id $b $seed $HX_ARGS)
      if [ "$rc" = 99 ]; then rf=SETUP; stop=1; vr "H-$id" "rc=99" NOT-RUN/SETUP; continue; fi
      if [ "$rc" != 0 ] && [ ! -s $L/$id.out ] && grep -q -E '^sudo:' $L/$id.err; then rf=SUDO; stop=1; vr "H-$id" "rc=$rc" NOT-RUN/SUDO; continue; fi
      smoke_class "$id" "$rc"
      v=$(hx_eval "$rc" "$SM_CLS" "$SM_AP" $want)
      vr "H-$id" "rc=$rc class=$SM_CLS applied=$SM_AP subtype=$SM_SUB wall_ms=$(cat $L/$id.wallms 2> /dev/null) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns)" "$v"
      case $v in
        HX-OK) if [ $want = 3 ]; then nok_d=$((nok_d + 1)); else nok_s=$((nok_s + 1)); fi;;
        HX-UNAPPLIED) nun=$((nun + 1));;
        HX-HERMITFAIL) nhf=$((nhf + 1)); stop=1;;
        *) noth=$((noth + 1)); stop=1;;
      esac
    done
  done
  if [ $noth -gt 0 ]; then w=HX-STOP
  elif [ $nhf -gt 0 ]; then w=HX-STOP-HERMITFAIL
  elif [ -n "$rf" ]; then w="HX-NOT-RUN/$rf"
  elif [ $nok_d -ge 1 ] && [ $nok_s -ge 1 ]; then w=HX-OK
  else w=HX-NOT-EXERCISED; fi
  V_HX=$w; vr HX "dpos_ok=$nok_d dstall_ok=$nok_s unapplied=$nun other=$noth hermit_fail=$nhf" "$w"; return 0; }
# pm_phase: INFORMATIONAL ONLY (6.8b), never graded, never read by overall(), no suffix: PM_N seeds of X1b's pinned counted condlock of arm P under the same Hermit wrapper at -r 60.
# Runs only if the exit probe finished without a STOP. Rows: rc, class, wall, virtual time, turns, ticks, the first dump line. Three consecutive cap or Hermit-failure runs end it.
pm_phase() { local i seed id rc ticks dump consec=0 stop=0 rf='' nrun=0 tsum=0 cls
  case $V_HX in HX-OK|HX-NOT-EXERCISED) ;; *) V_PM=PM-NOT-RUN/HX; vr PM "needs the exit probe to have run without a STOP or a not-run ($V_HX)" $V_PM; return 0;; esac
  cp $CTL/x1b-condlock-P $H/$PM_BIN 2> /dev/null
  cmp -s $CTL/x1b-condlock-P $H/$PM_BIN || { V_PM=PM-NOT-RUN/COPY; vr PM "$PM_BIN missing or the copy differs" $V_PM; return 0; }
  : > $R/pm.tsv
  for ((i = 0; i < PM_N; i++)); do
    seed=$((PM_FIRST + i)); id=pm-$seed
    if [ $stop = 1 ]; then vr "H-$id" skipped PM-SKIPPED; continue; fi
    if ! ok_to_start; then rf=$REFUSED; stop=1; vr "H-$id" refused "PM-NOT-RUN/$rf"; continue; fi
    rc=$(hermit_run $id $PM_BIN $seed $PM_ARGS)
    if [ "$rc" = 99 ]; then rf=SETUP; stop=1; vr "H-$id" "rc=99" PM-NOT-RUN/SETUP; continue; fi
    if [ "$rc" != 0 ] && [ ! -s $L/$id.out ] && grep -q -E '^sudo:' $L/$id.err; then rf=SUDO; stop=1; vr "H-$id" "rc=$rc" PM-NOT-RUN/SUDO; continue; fi
    smoke_class "$id" "$rc"; cls=$SM_CLS
    ticks=$(sed -n 's/.* ticks= *\([0-9][0-9]*\).*/\1/p' $L/$id.out | head -n 1)
    dump=$(grep -m 1 '^condlock: ' $L/$id.err | cut -c1-300)
    nrun=$((nrun + 1)); tsum=$((tsum + ${ticks:-0}))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$seed" "$rc" "$cls" "$(cat $L/$id.wallms 2> /dev/null)" "$(jnum $H/hout/$id.summary.json virttime_elapsed)" "$(jnum $H/hout/$id.summary.json sched_turns)" "${ticks:-NA}" "$SM_SUB" "${dump:-none}" >> $R/pm.tsv
    vr "H-$id" "rc=$rc class=$cls wall_ms=$(cat $L/$id.wallms 2> /dev/null) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns) ticks=${ticks:-NA} dump=[${dump:-none}]" INFO
    case $rc in 124|137|125|101) consec=$((consec + 1));; *) consec=0;; esac
    [ $consec -ge 3 ] && stop=1
  done
  V_PM=PM-INFO; [ -z "$rf" ] || V_PM="PM-INFO-REFUSED/$rf"
  vr PM "runs=$nrun of $PM_N; classes=[$(awk -F'\t' '{print $3}' $R/pm.tsv | LC_ALL=C sort | uniq -c | tr -s ' ' | tr '\n' ',')] ticks_sum=$tsum (information only, not graded)" "$V_PM"; return 0; }

# ---- O: the overall code (mechanical; first matching rule wins; the result is copied, not re-judged)
failcode() { local k=${1#FAIL-}; echo "X1C-FAIL-${k%%:*}/${k#*:}"; }
overall() { local v
  for v in "$V_BR" "$V_B1" "$V_B2" "$V_PIN" "$V_SMOKE"; do
    case $v in
      BR-OK|B1-OK|REPRO|NONREPRO-*|PINNED|SMOKE-OK) ;;
      NOT-RUN/*) echo "X1C-INCONCLUSIVE/${v#NOT-RUN/}"; return;;
      FAIL-*) failcode "$v"; return;;
      *) echo "X1C-UNKNOWN/$v"; return;;
    esac
  done
  case $V_HX in
    HX-OK) echo X1C-PASS;;
    HX-STOP) echo X1C-STOP-HEXIT;;
    HX-STOP-HERMITFAIL) echo X1C-STOP-HEXIT-HERMITFAIL;;
    HX-NOT-EXERCISED) echo X1C-HEXIT-NOT-EXERCISED;;
    HX-NOT-RUN/*) echo "X1C-PARTIAL/HEXIT-NOT-RUN-${V_HX#HX-NOT-RUN/}";;
    *) echo "X1C-UNKNOWN/$V_HX";;
  esac; }
res_suffix() { local s=
  case $V_B2 in NONREPRO-*) s="$s+$V_B2";; esac
  [ -z "$CTLDIFF" ] || s="$s+CTRL-DIFFER-$CTLDIFF"
  echo "$s"; }
side_effects() { local s=
  cmp -s $R/ldcache-before.txt $R/ldcache-post.txt || s="$s+LDCACHE-CHANGED"
  [ -s $R/leftover.txt ] && s="$s+LEFTOVER"
  cmp -s $R/tmp-before.txt $R/tmp-post.txt || s="$s+TMP-LISTING-DIFFERS"
  cmp -s $R/shm-before.txt $R/shm-post.txt || s="$s+SHM-LISTING-DIFFERS"
  cmp -s $R/crash-before.txt $R/crash-post.txt || s="$s+VAR-CRASH-DIFFERS"
  echo "$s$INCH$(res_suffix)"; }
overall_line() { printf 'OVERALL\t%s%s\tbranch=%s b1=%s b2=%s pin=%s smoke=%s hx=%s ctl=%s pm=%s\n' "$1" "$2" "$V_BR" "$V_B1" "$V_B2" "$V_PIN" "$V_SMOKE" "$V_HX" "${CTLDIFF:-identical}" "$V_PM"; }

# ---- test seam: the Mac stub tests source this file with X1C_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${X1C_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "X1C_TEST is set in a real run: refused" >&2; exit 5; fi
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
setup_copies
if branch_gate && pass1; then
  ctl_phase
  if pass2 && pin_phase; then
    if smoke_phase; then hermit_phase; pm_phase; else V_HX=HX-NOT-RUN/SMOKE; V_PM=PM-NOT-RUN/SMOKE; fi
  fi
fi
finish_up
overall_line "$(overall)" "$(side_effects)" >> $R/cases.tsv
echo DONE-OVERALL >> $R/steps.tsv
