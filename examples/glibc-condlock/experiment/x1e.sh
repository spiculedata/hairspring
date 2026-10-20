#!/usr/bin/env bash
# usage: taskset -c 0 setsid nohup timeout -k 30 3600 bash $R/bin/x1e.sh $R 1,2,3 <stamp> > $R/run.out 2>&1 < /dev/null &
# X1e: build the positive-control tree P2 on the box (the recipe of X1b's build_arm), compare its libc.a with X1b's U, link and pin P2's condlock and violation-only binaries,
# smoke them natively, and run the pre-registered Hermit probe (60 runs). Nothing is graded by this script.
# Functions marked "x1b:", "x1c:" or "x1d:" are byte-identical to x1b.sh, x1c.sh or x1d.sh; the others are new or adapted.
set -u
R=$1; CORES=$2; STAMP=$3; L=$R/logs; BIN=$R/bin
H=/dev/shm/x1e-glibc-$STAMP; HCORE=${CORES%%,*}
X1B=/path/to/x1b-glibc-20261017T1441Z   # EDIT: the work dir of x1b.sh; read only: the U tree and sysroot, the C binary, the record files
CK=$R/ck; CLK=$CK/examples/glibc-condlock; PASS1=$R/pass1; PASS2=$R/pass2; PIN=$R/pin; CTL=$R/ctl
HERMIT=/path/to/hermit   # EDIT: the pinned hermit binary (sha256 must equal HERMIT_SHA)
HERMIT_SHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
CPATH=/usr/bin:/bin   # PATH of the plain runs and of the root Hermit wrapper: never changed
STEPPATH=$CPATH       # PATH of the logged steps; phase T puts $R/tools/bin first (uid 1000 only)
T0=$(date +%s); BDL=$((T0 + 30 * 60)); NEWBY_B=$((T0 + 27 * 60)); NEWBY=$((T0 + 52 * 60))
RCAP_KB=4194304; HCAP_KB=51200; FREE_MIN_KB=10485760; SHM_MIN_KB=204800
# pins (spec section 3.1) and pre-registered settings (3.3, 6.7, 6.3); none is tuned after data
PIN_CLTAR=9eddf7b1d46ce88bc012b8eb76d0ba397b92ddacee8e538b5bb5bdada01f0014
PIN_CLS=5fbc06df3de905c7c67a745d1b8c93c62d886c182f98ac85f93c92433637d32a
PIN_X1B_CASES=0fd5a4a8f34c550d4e81d1d890ae1fa3fb40918e8f98bd02059f79a37e082eb1
PIN_X1B_STEPS=ea10be71b3e814b697d13db4068734cf7dfb805e06bb1b0b18b9093e0b485604
PIN_P2TAR=af929a88ac746b2946901b3ffe16e5f6feb55458604175baff4a1bba3180d1f9
PIN_PATCH=edb50446501aaf479b2859332a1db5131444eed0c74f4922e3c3379eab7470ec
U_FILE_SHA=55ce6de06fc59642cae8a82c63c683fb39856fdcf5a6cbc5b3940ba8cabc7e25    # nptl/pthread_cond_wait.c in U (and in X1b's U/src)
P2_FILE_SHA=2d28a8cd2f648b1beea405d3ae23c33c1a767c9a894985e9d3d37565358afd05   # the same file in P2 (U plus the one hunk; git blob e58d3b83)
MARKER=P2-POSITIVE-CONTROL-HUNK
pin_of() { case "$1_$2" in   # KIND ARM : the two X1b inputs this job reuses by hash
  LIBC_U) echo 7a3afc14df8b7c4d64278db94a559705469ba653347a0ba711d91ddbf909967e;;
  BIN_C) echo 9999bfac20b6c369f38103d06adb644117bc4e6b5dad286ded3dcbd524f5b70f;;
  *) echo unknown-pin;;
esac; }
SM1_ARGS="-n 2 -r 1 -tick-ms 50 -q 10"; SM2_ARGS="-n 8 -r 60 -tick-ms 50 -q 10"; SM2_REPS=3
PX_BIN=condlock-P2; PX_N=30; PX_CELLS="125 250"   # order of the cells; seeds: -r 125 from 319891, -r 250 from 319921
MOUNTS=/proc/mounts
RE_OUT='^condlock libc=2\.40 hdr=2\.40 rounds= *[0-9]+ signals= *[0-9]+ blocked1= *[0-9]+ maxb= *[0-9]+ maxstag= *[0-9]+ ticks= *[0-9]+ spanned= *[0-9]+( applied=[01])?$'
V_TOOLS=NOT-RUN/PRIOR; V_SRC=NOT-RUN/PRIOR; V_BUILD=NOT-RUN/PRIOR; V_CMP=NOT-RUN/PRIOR; V_BR=NOT-RUN/PRIOR; V_SDK=NOT-RUN/PRIOR; V_B1=NOT-RUN/PRIOR
V_B2=NOT-RUN/PRIOR; V_PIN=NOT-RUN/PRIOR; V_SMOKE=NOT-RUN/PRIOR; V_PX=PX-NOT-RUN/PRIOR
INCH=; KHDRDIFF=; STEPCLASS=A
PX_STOP=0; PX_CONSEC=0; PX_RF=

# x1d: note
note() { printf '%s\t%s\n' "$(date +%s)" "$*" >> $R/events.tsv; }
# x1b: vr (x1d's vr plus LASTV, the verdict just written)
LASTV=; BW=0
vr() { LASTV=$3; printf 'K\t%s\t%s\t%s\n' "$1" "$2" "$3" >> $R/cases.tsv; note "K $1 $3"; }   # id, raw value, verdict (LASTV = the verdict just written)
REFUSED=
# ok_to_start: x1b's, with this job's lines (52 min, build class 27 min, 4 GB, 50 MB, 10 GB). Returns 0, or 1 with REFUSED = STOP|HALT|BUDGET|PAUSED|DISK|PROC.
ok_to_start() { local now; now=$(date +%s); REFUSED=
  [ -e $R/STOP ] && { REFUSED=STOP; note STOPPED; return 1; }
  [ -e $R/HALT ] && { REFUSED=HALT; note "HALTED: $(cat $R/HALT)"; return 1; }
  [ "$now" -ge $NEWBY ] && { REFUSED=BUDGET; note "BUDGET: no new step after 52 min"; return 1; }
  [ "$STEPCLASS" = B ] && [ "$now" -ge $NEWBY_B ] && { REFUSED=BUDGET; note "BUDGET: no new build step after 27 min"; return 1; }
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && { REFUSED=PAUSED; note "PAUSED: heavy job started"; return 1; }
  [ "$(du -sk $R | cut -f1)" -gt $RCAP_KB ] && { REFUSED=DISK; note "DISK: $R over 4 GB cap"; return 1; }
  [ -d $H ] && [ "$(du -sk $H 2> /dev/null | cut -f1)" -gt $HCAP_KB ] && { REFUSED=DISK; note "DISK: $H over 50 MB cap"; return 1; }
  [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -lt $FREE_MIN_KB ] && { REFUSED=DISK; note "DISK: less than 10 GB free"; return 1; }
  for _ in 1 2 3 4 5; do { procs_with_dir $R; procs_with_dir $H; } > $R/procs-now.txt; [ -s $R/procs-now.txt ] || break; sleep 1; done
  [ -s $R/procs-now.txt ] && { REFUSED=PROC; note "REFUSED: a process has the work dir in its arguments: $(head -n 1 $R/procs-now.txt | cut -c1-200)"; return 1; }
  return 0; }
# x1b: bv
bv() { local id=$1 rc=$2 fail=$3 tmo=${4:-BUILD-TIMEOUT}
  if [ "$rc" = 99 ]; then vr $id "rc=99 refused" "NOT-RUN/${REFUSED:-UNKNOWN}"
  elif [ "$rc" = 124 ] || [ "$rc" = 137 ]; then vr $id rc=$rc "$tmo"
  else vr $id rc=$rc "$fail"; fi; }
# x1d: snapshot
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
# x1d: leftovers
leftovers() { local p e; pgrep -s $SID > $R/sess.tmp   # the driver ($$) and the timeout that owns the session ($PPID) are excluded
  while read -r p; do [ "$p" = $$ ] || [ "$p" = $PPID ] || echo "session $p $(tr '\0' ' ' < /proc/$p/cmdline 2> /dev/null)"; done < $R/sess.tmp
  for p in /proc/[0-9]*; do e=$(readlink $p/exe 2> /dev/null); case $e in $R/*|$H/*) echo "exe-scan ${p#/proc/} $e";; esac; done
  ps -eo pid=,user=,sid=,args= | RP="$H/" awk 'index($0,ENVIRON["RP"]) { print "root-scan " $0 }'; }
# x1d: procs_with_dir
# every process of ANY user, in ANY session, whose arguments contain "$1/", except the driver's ancestor chain ANC and its own direct children
procs_with_dir() { ps -eo pid=,ppid=,user=,sid=,args= | RP="$1/" awk -v me="$$" -v anc="$ANC" 'index(anc, " " $1 " ") == 0 && $2!=me && index($0,ENVIRON["RP"])'; }
# x1d: step
step() { # id timeout cwd cmd... : one logged step, clean env, pinned cores, nice 10
  local id=$1 to=$2 cwd=$3 t0 rc; shift 3
  ok_to_start || return 99
  t0=$(date +%s.%N)
  ( ulimit -c 0; cd "$cwd" && env -i PATH=$STEPPATH HOME=$R LANG=C LC_ALL=C TMPDIR=$R/tmp nice -n 10 taskset -c $CORES timeout -k 30 "$to" "$@" ) > $L/$id.log 2>&1
  rc=$?
  printf '%s\t%s\t%s\t%s\n' "$id" "$rc" "$(awk -v a=$t0 -v b="$(date +%s.%N)" 'BEGIN{printf "%.1f",b-a}')" "$*" >> $R/steps.tsv; return $rc; }
# x1d: runone
runone() { local id=$1 tmo=$2; shift 2   # one plain run; prints the exit status; stdout and stderr to $L/<id>.out and .err
  ( ulimit -c 0; cd $R && env -i PATH=$CPATH HOME=$R LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$tmo" "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; echo $?; }
# x1d: sha_of
sha_of() { sha256sum "$1" 2> /dev/null | cut -d' ' -f1; }
# x1d: jnum
jnum() { sed -n "s/^ *\"$2\": *\([0-9][0-9]*\).*/\1/p" "$1" 2> /dev/null | head -n 1; }   # the first integer after "key": in a Hermit summary
# x1b: bto
bto() { echo $(( BDL - $(date +%s) )); }
# end_checks: x1d's pattern for this job's inputs (the reused X1b inputs, the tars, the tools, the Hermit pin, the pin table, the helpers) hashed again at the end
end_checks() { local n s bad='' pinbad='' inbad=''
  INCH=
  if [ -e $R/tars.sha ]; then (cd $R && sha256sum -c tars.sha > $R/end-tars.txt 2>&1) || inbad=1; fi
  if [ -e $R/tools.sha ]; then (cd $R && sha256sum -c tools.sha > $R/end-tools.txt 2>&1) || inbad=1; fi
  if [ -e $CLK/scripts/classify.sh ]; then [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || inbad=1; fi
  [ -z "$inbad" ] || INCH="$INCH+INPUT-CHANGED"
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || INCH="$INCH+HERMIT-CHANGED"
  if [ -d $X1B ]; then
    [ "$(sha_of $X1B/U/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC U)" ] || bad=1
    [ "$(sha_of $X1B/U/src/nptl/pthread_cond_wait.c)" = $U_FILE_SHA ] || bad=1
    [ "$(sha_of $X1B/ck/b-C/condlock)" = "$(pin_of BIN C)" ] || bad=1
    [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || bad=1
    [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || bad=1
  else bad=1; fi
  [ -z "$bad" ] || INCH="$INCH+X1B-CHANGED"
  if [ -s $R/pin-table.tsv ]; then
    while IFS=$'\t' read -r n s _; do [ "$(sha_of $PIN/$n)" = "$s" ] || pinbad=1; done < $R/pin-table.tsv
    [ -z "$pinbad" ] || INCH="$INCH+PIN-CHANGED"
  fi
  (cd $BIN && sha256sum x1e.sh tools.sh) > $R/end-helpers.txt 2>&1; }
# x1d: rootowned
# rootowned: everything under $H that is not owned by ubuntu (Hermit runs as root), listed and LEFT IN PLACE
rootowned() { find $H ! -user ubuntu -printf '%u %m %p\n' > $R/rootowned.txt 2>&1; vr Z-rootowned "$(grep -c . $R/rootowned.txt) lines (root-owned entries and permission errors, left in place)" INFO; }
# x1d: finish_up
finish_up() { end_checks; snapshot post; leftovers > $R/leftover.txt; rootowned; note "LEFTOVER=$(wc -l < $R/leftover.txt) DONE"; echo DONE >> $R/steps.tsv; }
# on_signal, gate_fail: x1d's with the X1E codes
on_signal() { trap '' TERM INT HUP; echo SIGNAL > $R/HALT; note "SIGNAL: the driver was told to stop; HALT written"; finish_up
  printf 'OVERALL\tX1E-INCONCLUSIVE/SIGNAL\n' >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 6; }
gate_fail() { vr "G-$1" "gate failed" GATE-FAIL; note "GATE-FAIL $1"; echo "GATE-FAIL $1" >> $R/steps.tsv
  finish_up; printf 'OVERALL\tX1E-NOT-STARTED/%s\n' "$1" >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 5; }

# ---- names, per-name specification, compiler line (x1c's, reduced to the two binaries of this job and pointed at the P2 sysroot)
all_names() { echo condlock-P2; echo condlock-novio-P2; echo tprobe-P2; }
smoke_names() { echo condlock-P2; echo condlock-novio-P2; }   # tprobe-P2 is built, isolated and pinned (GLG-P copies it) but never run natively here
# x1c: armdef
armdef() { case $1 in C) echo CONDLOCK_ARM_C;; *) echo CONDLOCK_ARM_U;; esac; }   # P has the U layout (X1b); so has P2
# spec_of NAME : SP_ARM (sysroot arm), SP_KIND (sdk|novio), SP_TARGET (under BUILD), SP_TOK (the extra define, or - for the counted build); returns 1 for an unknown name
spec_of() { local n=$1
  case $n in
    condlock-P2) SP_ARM=P2; SP_KIND=sdk; SP_TARGET=condlock; SP_TOK=-; return 0;;
    condlock-novio-P2) SP_ARM=P2; SP_KIND=novio; SP_TARGET=condlock-novio; SP_TOK=-DCONDLOCK_NO_SDK; return 0;;
    tprobe-P2) SP_ARM=P2; SP_KIND=tprobe; SP_TARGET=tprobe; SP_TOK=-; return 0;;
  esac; return 1; }
cc_of() { local S=$R/$1/sysroot; echo "gcc --sysroot=$S -B$S/usr/lib/ -static -L$S/usr/lib -Wl,-t,-t"; }   # x1b's CC line byte for byte, the sysroot being this job's own P2 tree
# x1c: inc_viol
inc_viol() { # log sysroot : absolute include paths in a gcc -M log that are neither in the sysroot, nor gcc's own, nor the condlock tree
  tr -s ' \\' '\n' < $L/$1.log | grep '^/' | grep -v -e "^$2/" -e "^$(realpath "$2")/" -e '^/usr/lib/gcc/x86_64-linux-gnu/13/' -e "^$CK/" | wc -l | tr -d ' '; }
# x1c: branch_of
branch_of() { # preprocessed cl_condraw.c : which layout branch is live (the silent ARM_NONE fallback shows as 0 and 0)
  echo "arm_u=$(grep -c 'raw.g_refs\[0\] = c->__data.__g_refs\[0\]' $1) arm_c=$(grep -c 'raw.arm = CL_ARM_C;' $1) value64=$(grep -c 'raw.wseq = c->__data.__wseq.__value64' $1)"; }
# x1c: iso_trace
iso_trace() { # sysroot log tag : prints the number of violations; raw offenders go to $L/iso-<tag>.txt
  local S=$1 log=$2 t=$3 pat
  pat='(crt1|crti|crtn|Scrt1|rcrt1|gcrt1)\.o|lib(c|m|pthread|dl|rt|util|resolv|anl|mvec|c_nonshared)\.(a|so)|ld-linux'
  grep -v '^attempt to open .* failed' $log > $L/iso-$t.tr
  { grep -E "$pat" $L/iso-$t.tr | grep -vF "$S/"; grep -E '/(usr/)?lib(64)?/x86_64-linux-gnu/' $L/iso-$t.tr; } > $L/iso-$t.txt
  grep -c . $L/iso-$t.txt; }
# x1c: libc_hits
libc_hits() { grep -cF -e "$2/usr/lib/libc.a(" -e "($2/usr/lib/libc.a)" "$1"; }   # both trace forms: path(member) and (path)member

# ---- G: gates on the reused X1b work dir and on this job's inputs (no repair), then the extraction of condlock.tar
gates() {
  [ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/gate-sudo.log 2>&1 || gate_fail SUDO
  if grep ' /dev/shm ' $MOUNTS 2> /dev/null | grep -qw noexec; then gate_fail NOEXEC; fi
  { [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -ge $FREE_MIN_KB ] && [ "$(df -P -k /dev/shm | awk 'NR==2{print $4}')" -ge $SHM_MIN_KB ]; } || gate_fail FREE
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || gate_fail HERMIT-SHA
  [ "$(gcc -dumpfullversion 2> /dev/null)" = 13.3.0 ] || gate_fail GCC-VERSION
  [ -d $X1B ] || gate_fail REUSE-DIR
  [ "$(sha_of $X1B/cases.tsv)" = $PIN_X1B_CASES ] || gate_fail REUSE-RECORD-CASES
  [ "$(sha_of $X1B/steps.tsv)" = $PIN_X1B_STEPS ] || gate_fail REUSE-RECORD-STEPS
  [ "$(sha_of $X1B/U/src/nptl/pthread_cond_wait.c)" = $U_FILE_SHA ] || gate_fail REUSE-SRC-U
  [ "$(sha_of $X1B/U/sysroot/usr/lib/libc.a)" = "$(pin_of LIBC U)" ] || gate_fail REUSE-LIBC-U
  [ "$(sha_of $X1B/ck/b-C/condlock)" = "$(pin_of BIN C)" ] || gate_fail REUSE-BIN-C
  [ "$(sha_of $R/condlock.tar)" = $PIN_CLTAR ] || gate_fail CLTAR-SHA
  [ "$(sha_of $R/glibc-P2.tar)" = $PIN_P2TAR ] || gate_fail P2TAR-SHA
  [ "$(sha_of $R/positive-control-p2.patch)" = $PIN_PATCH ] || gate_fail PATCH-SHA
  vr G-reuse "the X1b work dir is the one X1b recorded (U src file, U libc.a, C binary, cases.tsv, steps.tsv); the P2 tar, the patch and condlock.tar are the pinned ones" OK; return 0; }
# x1c: ck_extract
ck_extract() { local rc
  mkdir -p $CK
  step C-x-ck 300 $R tar -xf condlock.tar -C $CK; rc=$?; [ $rc = 0 ] || gate_fail CK-EXTRACT
  [ "$(sha_of $CLK/scripts/classify.sh)" = $PIN_CLS ] || gate_fail CLASSIFY-SHA
  if grep -qx 'SDK_COMMIT=a4131e49680fd80edf66d560a5cbf4be54de0cd7' $CLK/PINS && grep -qx 'SDK_TREE=f5d13d7773d6211e9ebb2283f774bc4b00cd6013' $CLK/PINS \
    && [ -f $CLK/src/cl_condraw.c ] && [ -f $CLK/src/cl_plant.h ] && [ -f $CLK/src/main.c ] && [ -f $CLK/Makefile ] && [ -f $CK/sdks/c/Makefile ]; then
    vr C-layout "$(find $CK -type f | wc -l | tr -d ' ') files" OK; return 0; fi
  gate_fail CK-LAYOUT; }
setup_copy() { mkdir -p $PASS1 $PASS2 $PIN $CTL $R/tmp; return 0; }

# ---- T: private bison and m4 (x1b's tools_phase, byte for byte)
# x1b: tools_phase
tools_phase() { local rc TB=$R/tools-build TS=$R/tools-src bv_line mv_line
  mkdir -p $TB/m4 $TB/bison $TS $R/tools
  step T-sum 120 $R sha256sum -c tools.sha; rc=$?; [ $rc = 0 ] || { bv T-sum $rc TOOLS-FAIL-SUM TOOLS-FAIL-SUM; return 1; }
  step T-x-m4 300 $TS tar -xzf $R/m4-1.4.19.tar.gz; rc=$?; [ $rc = 0 ] || { bv T-x-m4 $rc TOOLS-FAIL-EXTRACT TOOLS-FAIL-EXTRACT; return 1; }
  step T-x-bison 300 $TS tar -xzf $R/bison-3.8.2.tar.gz; rc=$?; [ $rc = 0 ] || { bv T-x-bison $rc TOOLS-FAIL-EXTRACT TOOLS-FAIL-EXTRACT; return 1; }
  STEPCLASS=B step T-m4-cfg 900 $TB/m4 $TS/m4-1.4.19/configure --prefix=$R/tools --disable-nls; rc=$?; [ $rc = 0 ] || { bv T-m4-cfg $rc TOOLS-FAIL-M4-CONFIGURE; return 1; }
  STEPCLASS=B step T-m4-mk 1800 $TB/m4 make -j3; rc=$?; [ $rc = 0 ] || { bv T-m4-mk $rc TOOLS-FAIL-M4-MAKE; return 1; }
  STEPCLASS=B step T-m4-in 300 $TB/m4 make install; rc=$?; [ $rc = 0 ] || { bv T-m4-in $rc TOOLS-FAIL-M4-INSTALL; return 1; }
  STEPPATH=$R/tools/bin:$CPATH   # bison's configure finds the private m4 through PATH
  STEPCLASS=B step T-bison-cfg 900 $TB/bison $TS/bison-3.8.2/configure --prefix=$R/tools --disable-nls M4=$R/tools/bin/m4; rc=$?; [ $rc = 0 ] || { bv T-bison-cfg $rc TOOLS-FAIL-BISON-CONFIGURE; return 1; }
  STEPCLASS=B step T-bison-mk 2400 $TB/bison make -j3; rc=$?; [ $rc = 0 ] || { bv T-bison-mk $rc TOOLS-FAIL-BISON-MAKE; return 1; }
  STEPCLASS=B step T-bison-in 300 $TB/bison make install; rc=$?; [ $rc = 0 ] || { bv T-bison-in $rc TOOLS-FAIL-BISON-INSTALL; return 1; }
  # verify, with the PATH every later step will use: the first bison and m4 on it must be the private ones, at the pinned versions, and bison must satisfy glibc's >= 2.7
  step T-ver 60 $R sh -c 'command -v bison; command -v m4; bison --version | head -n 1; m4 --version | head -n 1' ; rc=$?; [ $rc = 0 ] || { bv T-ver $rc TOOLS-FAIL-VERIFY TOOLS-FAIL-VERIFY; return 1; }
  cp $L/T-ver.log $R/tools-versions.txt
  bv_line=$(sed -n 3p $L/T-ver.log); mv_line=$(sed -n 4p $L/T-ver.log)
  vr T-versions "$(sed -n 1p $L/T-ver.log) | $(sed -n 2p $L/T-ver.log) | $bv_line | $mv_line" INFO
  [ "$(sed -n 1p $L/T-ver.log)" = "$R/tools/bin/bison" ] && [ "$(sed -n 2p $L/T-ver.log)" = "$R/tools/bin/m4" ] \
    && [ "$bv_line" = "bison (GNU Bison) 3.8.2" ] && [ "$mv_line" = "m4 (GNU M4) 1.4.19" ] \
    && [ "$(printf '%s\n2.7\n' "${bv_line##* }" | sort -V | head -n 1)" = 2.7 ] || { vr T-ver "$bv_line | $mv_line" TOOLS-FAIL-VERIFY; return 1; }
  vr T-done "bison 3.8.2, m4 1.4.19 in $R/tools" TOOLS-BUILT; return 0; }

# ---- A: sources. X1b's src_phase, for the one tree P2: the tars are verified again, the tree is compared with X1b's U/src (read only), the hunk is compared with the shipped patch
src_phase() {
  local n want rc uf pf mp mu
  step A-tarsum 300 $R sha256sum -c tars.sha; rc=$?; [ $rc = 0 ] || { bv A-tarsum $rc SRC-FAIL SRC-FAIL; return 1; }
  mkdir -p $R/P2; step A-x-P2 900 $R tar -xf glibc-P2.tar -C $R/P2; rc=$?; [ $rc = 0 ] || { bv A-x-P2 $rc SRC-FAIL SRC-FAIL; return 1; }
  n=$(find $R/P2/src \( -type f -o -type l \) | wc -l | tr -d ' '); want=$(awk '$1=="P2"{print $2}' $R/counts.txt)
  vr A-count-P2 "$n want=$want" "$([ "$n" = "$want" ] && echo OK || echo SRC-FAIL)"; [ "$n" = "$want" ] || return 1
  (cd $R/P2/src && sha256sum nptl/pthread_cond_broadcast.c nptl/pthread_cond_common.c nptl/pthread_cond_signal.c nptl/pthread_cond_wait.c) > $R/srcfiles.txt
  vr A-U-line "$(grep -cF 'uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);' $X1B/U/src/nptl/pthread_cond_wait.c)" INFO-exact-line-count-in-U
  vr A-P2-line "$(grep -cF 'uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);' $R/P2/src/nptl/pthread_cond_wait.c)" INFO-exact-line-count-in-P2
  # P1: U and P2 differ in exactly one file (X1b's U/src is the U tree, which X1b's build has run python scripts from, so a bytecode cache directory may exist in it: __pycache__ is excluded and the unfiltered diff is a row; the prefixes are cut from diff's output)
  diff -rq $X1B/U/src $R/P2/src | sed -e "s|$X1B/||g" -e "s|$R/||g" > $R/srcdiff-UP2-raw.txt
  vr A-srcdiff-raw "$(grep -c . $R/srcdiff-UP2-raw.txt) lines unfiltered, $(grep -c __pycache__ $R/srcdiff-UP2-raw.txt) of them name __pycache__" INFO
  diff -rq -x __pycache__ $X1B/U/src $R/P2/src | sed -e "s|$X1B/||g" -e "s|$R/||g" > $R/srcdiff-UP2.txt
  vr A-srcdiff-UP2 "$(tr '\n' ' ' < $R/srcdiff-UP2.txt | cut -c1-300)" "$([ "$(cat $R/srcdiff-UP2.txt)" = "Files U/src/nptl/pthread_cond_wait.c and P2/src/nptl/pthread_cond_wait.c differ" ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P2: that file is the pinned one in U and in P2
  uf=$(sha256sum $X1B/U/src/nptl/pthread_cond_wait.c | cut -d' ' -f1); pf=$(sha256sum $R/P2/src/nptl/pthread_cond_wait.c | cut -d' ' -f1)
  vr A-filesha "U=$uf P2=$pf" "$([ "$uf" = $U_FILE_SHA ] && [ "$pf" = $P2_FILE_SHA ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P3: the marker comment is counted once in P2 and zero times in U
  mp=$(grep -cF "$MARKER" $R/P2/src/nptl/pthread_cond_wait.c); mu=$(grep -cF "$MARKER" $X1B/U/src/nptl/pthread_cond_wait.c)
  vr A-marker "P2=$mp U=$mu" "$([ "$mp" = 1 ] && [ "$mu" = 0 ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P4: diff of U's file against P2's reproduces exactly the added lines of the shipped patch (one range line, no removed line)
  diff $X1B/U/src/nptl/pthread_cond_wait.c $R/P2/src/nptl/pthread_cond_wait.c > $R/hunk.diff
  grep '^+' $R/positive-control-p2.patch | grep -v '^+++' | cut -c2- > $R/hunk-patch.txt
  grep '^>' $R/hunk.diff | cut -c3- > $R/hunk-diff.txt
  vr A-hunk "range: $(grep '^[0-9]' $R/hunk.diff | tr '\n' ' ') added=$(grep -c '^>' $R/hunk.diff) removed=$(grep -c '^<' $R/hunk.diff) patch-added=$(grep -c . $R/hunk-patch.txt)+blank" \
    "$(cmp -s $R/hunk-patch.txt $R/hunk-diff.txt && [ "$(grep -c '^<' $R/hunk.diff)" = 0 ] && [ "$(grep -c '^[0-9]' $R/hunk.diff)" = 1 ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  [ -x $R/P2/src/configure ] || { vr A-configure missing SRC-FAIL; return 1; }
  return 0; }

# ---- K: kernel headers (x1b's khdr_phase, byte for byte), then a comparison of the manifest with X1b's
# x1b: khdr_phase
khdr_phase() { local rc
  step K-copy 300 $R bash -c "mkdir -p $R/khdr && cp -a /usr/include/linux $R/khdr/linux && cp -a /usr/include/asm-generic $R/khdr/asm-generic && cp -a /usr/include/x86_64-linux-gnu/asm $R/khdr/asm"; rc=$?
  [ $rc = 0 ] || { bv K-copy $rc KHDR-FAIL KHDR-FAIL; return 1; }
  (cd $R/khdr && find . -type f | sort | xargs sha256sum | sha256sum) > $R/khdr.manifest 2>&1
  dpkg-query -W linux-libc-dev > $R/khdr.pkg 2>&1; return 0; }
# khdr_cmp: the kernel headers copied here are the ones X1b's build used (its khdr.manifest); a difference is a suffix, not a stop
khdr_cmp() { if cmp -s $R/khdr.manifest $X1B/khdr.manifest; then vr X-khdr-x1b "equal=yes" INFO; else KHDRDIFF=1; vr X-khdr-x1b "equal=no ($(cat $X1B/khdr.manifest 2>&1 | cut -c1-80) | $(cut -c1-80 $R/khdr.manifest))" INFO; fi; return 0; }

# ---- B: build the tree P2 (x1b's build_arm, byte for byte; arm name P2, so the file prefix map is $R/P2=/g0)
# x1b: build_arm
build_arm() { local a=$1 t0 to rc f S miss=; S=$R/$a/sysroot; t0=$(date +%s); mkdir -p $R/$a/build
  STEPCLASS=B step cfg-$a 1500 $R/$a/build ../src/configure CC=gcc "CFLAGS=-O2 -g -U_FORTIFY_SOURCE -ffile-prefix-map=$R/$a=/g0" MAKEINFO=: --prefix=/usr --libdir=/usr/lib --with-headers=$R/khdr --disable-werror --disable-profile --disable-nscd --without-selinux; rc=$?
  [ $rc = 0 ] || { bv B-cfg-$a $rc BUILD-FAIL-CONFIGURE; return 1; }
  to=$(bto); [ "$to" -gt 300 ] || { vr B-mk-$a "left=${to}s" NOT-RUN/BUDGET; return 1; }
  STEPCLASS=B step mk-$a "$to" $R/$a/build make -j3; rc=$?
  [ $rc = 0 ] || { bv B-mk-$a $rc BUILD-FAIL-MAKE; return 1; }
  to=$(bto); [ "$to" -gt 120 ] || { vr B-in-$a "left=${to}s" NOT-RUN/BUDGET; return 1; }
  STEPCLASS=B step in-$a "$to" $R/$a/build make install install_root=$S; rc=$?
  [ $rc = 0 ] || { bv B-in-$a $rc BUILD-FAIL-INSTALL; return 1; }
  step khdr-in-$a 300 $R cp -a $R/khdr/. $S/usr/include/; rc=$?; [ $rc = 0 ] || { bv B-khdr-$a $rc BUILD-FAIL-KHDR; return 1; }   # make install copies only glibc's own headers
  for f in usr/lib/libc.a usr/lib/crt1.o usr/lib/crti.o usr/lib/crtn.o usr/include/gnu/libc-version.h usr/include/pthread.h usr/include/bits/thread-shared-types.h; do [ -f $S/$f ] || miss="$miss $f"; done
  if [ -n "$miss" ]; then find $S -name 'libc.a' -o -name 'crt1.o' > $R/$a.find.txt 2>&1; vr B-files-$a "missing:$miss" BUILD-FAIL-LAYOUT; return 1; fi
  vr B-hdr-$a "$(grep -E '#define +__GLIBC_MINOR__' $S/usr/include/features.h)" INFO
  vr B-libc-$a "$(sha256sum $S/usr/lib/libc.a | cut -d' ' -f1)" INFO
  BW=$(( $(date +%s) - t0 )); vr B-wall-$a "${BW}s" BUILT; return 0; }

# ---- D: the graded disassembly checks of G0p2 rev 2, run here on the box's stripped P2 pthread_cond_wait.o. dis_order, order_status and loop_perpass_status are
# byte-identical to gp2-in.sh (compared function by function before the run).
MIN_PASS_BRANCHES=10000     # graded: conditional branches per pass in every inlined copy (design 4.1: one chaos cap)
# g0p2: dis_order
dis_order() { # dump : "S" lines (one per pause loop) and "D" lines (the loop body and its neighbourhood) from a GNU objdump -d --no-show-raw-insn listing
  # S <function> fp lp cas hoist after head tail body cbr jmp const:
  #   fp/lp first/last pause instruction of the loop (instruction numbers within the function); cas the last "lock cmpxchg" before the loop (0 = none);
  #   hoist 1 when a load "mov 0x8(%reg),%reg" (the __g1_start pattern, reg not rsp/rbp) lies between that cas and the loop; after 1 when such a load follows the loop;
  #   head/tail the addresses of the innermost backward branch target and of that branch around the pause ("-" when none); body the instruction count between them;
  #   cbr the conditional branch instructions in the body (-1 when no loop could be delimited); jmp the unconditional ones; const 1 when 0x9c40 or 0x9c3f appears in
  #   the body or in the 6 instructions before it; kconst/step/perpass the loop constant, the loop step and the branches per pass (K / S x cbr) of the ONE recognised
  #   counter form (see perpass() below), perpass=-1 for anything else.
  awk '
    function h2n(s,   i, v) { v = 0; for (i = 1; i <= length(s); i++) v = v * 16 + index("0123456789abcdef", substr(s, i, 1)) - 1; return v }
    function isg1(s) { return s ~ /^mov[ \t]+0x8\(%r(ax|bx|cx|dx|si|di|8|9|1[0-5])\),%r[a-z0-9]+$/ }
    function iscbr(s,   m) { m = s; sub(/[ \t].*/, "", m); return (m ~ /^j/ && m != "jmp") || m ~ /^loop/ }
    function isjmp(s,   m) { m = s; sub(/[ \t].*/, "", m); return m == "jmp" }
    function enc(p,   k, nf, f, tg, found, best, t) {
      # the innermost backward branch (conditional or not) that encloses instruction p: its instruction number (0 = none); its target address goes to ENCHD
      found = 0; best = 0; t = 0; ENCHD = 0
      for (k = 1; k <= n; k++) {
        if (!(iscbr(ins[k]) || isjmp(ins[k]))) continue
        nf = split(ins[k], f, /[ \t]+/)
        if (nf < 2 || f[2] !~ /^[0-9a-f]+$/) continue
        tg = h2n(f[2])
        if (tg <= A[p] && A[k] >= A[p] && tg <= A[k] && (!found || A[k] - tg < best)) { found = 1; best = A[k] - tg; ENCHD = tg; t = k }
      }
      return t
    }
    function r32ok(s) { return s ~ /^%(e[a-d]x|e[sd]i|r(8|9|1[0-5])d)$/ }
    function immok(s) { return s ~ /^\$0x[0-9a-f]+$/ }
    function perpass(bi, tl, cb,   i, nf, f, g, rg, r64, ss, kk, q, ok) {
      # branches per pass for the ONE recognised counter form, else -1 (unrecognised): the body is only "pause" instructions, then "sub $S,R" (R a 32-bit register)
      # immediately followed by the backward "jne"; the nearest earlier instruction (nops skipped, up to 12 back) that mentions R or its 64-bit name is "mov $K,R"; K % S == 0.
      # branches per pass = (K / S) x conditional branches in the body. Counting up, "cmp $K", "dec", "add $-n", "lea", other branch kinds or other body code: unrecognised.
      PK = 0; PS = 0
      if (tl - bi < 2) return -1
      if (ins[tl] !~ /^jne[ \t]/) return -1
      for (i = bi; i < tl - 1; i++) if (ins[i] !~ /^pause$/) return -1
      nf = split(ins[tl - 1], f, /[ \t]+/)
      if (nf != 2 || f[1] != "sub") return -1
      split(f[2], g, ",")
      if (!immok(g[1]) || !r32ok(g[2])) return -1
      ss = h2n(substr(g[1], 4)); rg = g[2]
      if (ss < 1) return -1
      r64 = rg
      if (r64 ~ /^%e/) sub(/^%e/, "%r", r64); else sub(/d$/, "", r64)
      ok = 0; kk = 0
      for (i = bi - 1; i >= 1 && i >= bi - 12; i--) {
        if (ins[i] ~ /^nop/) continue
        if (index(ins[i], rg) == 0 && index(ins[i], r64) == 0) continue
        nf = split(ins[i], f, /[ \t]+/)
        if (nf == 2 && f[1] == "mov") { split(f[2], g, ","); if (g[2] == rg && immok(g[1])) { kk = h2n(substr(g[1], 4)); ok = 1 } }
        break
      }
      if (!ok || kk < 1 || kk % ss != 0) return -1
      PK = kk; PS = ss
      return int(kk / ss) * cb
    }
    function flush(   i, j, k, nl, fp, lp, lim, c, h, a, hd, tl, bi, cb, jm, cs, lo, hi, hds, tls, pp) {
      split("", ps); split("", E); split("", EH); nl = 0
      for (i = 1; i <= n; i++) if (ins[i] ~ /^pause/) ps[++nl] = i
      for (j = 1; j <= nl; j++) { E[j] = enc(ps[j]); EH[j] = ENCHD }
      j = 1
      while (j <= nl) {
        # one loop = the pauses that share the same enclosing backward branch (an unrolled loop has several); a pause with none is a loop of its own
        fp = ps[j]; lp = fp; k = j
        if (E[j] != 0) while (k < nl && E[k + 1] == E[j]) { k++; lp = ps[k] }
        hd = EH[j]; tl = E[j]
        lim = (k < nl) ? ps[k + 1] : n + 1
        c = 0
        for (i = fp - 1; i >= 1; i--) if (ins[i] ~ /^lock[ \t]+cmpxchg/) { c = i; break }
        h = 0
        if (c > 0) for (i = c + 1; i < fp; i++) if (isg1(ins[i])) h = 1
        a = 0
        for (i = lp + 1; i < lim; i++) if (isg1(ins[i])) { a = 1; break }
        if (tl > 0) {
          bi = 1
          for (i = 1; i <= n; i++) if (A[i] >= hd) { bi = i; break }
          cb = 0; jm = 0
          for (i = bi; i <= tl; i++) { if (iscbr(ins[i])) cb++; else if (isjmp(ins[i])) jm++ }
          cs = 0; lo = bi - 6; if (lo < 1) lo = 1
          for (i = lo; i <= tl; i++) if (ins[i] ~ /0x9c40|0x9c3f/) cs = 1
          pp = perpass(bi, tl, cb)
          hds = sprintf("%x", hd); tls = sprintf("%x", A[tl])
          printf "S %s fp=%d lp=%d cas=%d hoist=%d after=%d head=%s tail=%s body=%d cbr=%d jmp=%d const=%d kconst=%d step=%d perpass=%d\n", name, fp, lp, c, h, a, hds, tls, tl - bi + 1, cb, jm, cs, PK, PS, pp
          lo = bi - 6; if (lo < 1) lo = 1
          hi = tl + 8; if (hi > n) hi = n
        } else {
          cb = -1
          printf "S %s fp=%d lp=%d cas=%d hoist=%d after=%d head=- tail=- body=0 cbr=-1 jmp=0 const=0 kconst=0 step=0 perpass=-1\n", name, fp, lp, c, h, a
          lo = fp - 8; if (lo < 1) lo = 1
          hi = lp + 8; if (hi > n) hi = n
        }
        printf "D == %s loop at instructions %d-%d (lock cmpxchg at %d); conditional branches per iteration: %d (-1 = loop body not delimited); loop constant %d, step %d, branches per pass %d (-1 = counter form not recognised)\n", name, fp, lp, c, cb, (tl > 0 ? PK : 0), (tl > 0 ? PS : 0), (tl > 0 ? pp : -1)
        for (i = lo; i <= hi; i++) printf "D %s\n", raw[i]
        j = k + 1
      }
    }
    /^[0-9a-f]+ <.*>:$/ { flush(); n = 0; name = $2; next }
    /^[ \t]+[0-9a-f]+:[ \t]/ {
      t = $0; sub(/^[ \t]+[0-9a-f]+:[ \t]+/, "", t); sub(/[ \t]+$/, "", t)
      ad = $0; sub(/^[ \t]+/, "", ad); sub(/:.*/, "", ad)
      n++; ins[n] = t; raw[n] = $0; A[n] = h2n(ad)
    }
    END { flush() }
  ' "$1"; }
# g0p2: order_status
order_status() { # S-lines : ok / hoisted / pattern / no-loops
  local f=$1 nl h nc na
  nl=$(grep -c . "$f"); h=$(grep -c ' hoist=1 ' "$f"); nc=$(grep -c ' cas=0 ' "$f"); na=$(grep -c ' after=0 ' "$f")
  if [ "$nl" = 0 ]; then echo "no-loops"
  elif [ "$h" != 0 ]; then echo "hoisted loops=$nl hoist=$h"
  elif [ "$nc" != 0 ] || [ "$na" != 0 ]; then echo "pattern loops=$nl no-cas=$nc no-load-after=$na"
  else echo "ok loops=$nl"; fi; }
# g0p2: loop_perpass_status
loop_perpass_status() { # S-lines : graded: branches per pass >= MIN_PASS_BRANCHES in every inlined copy; any unrecognised counter form (perpass=-1) is reported as such
  local f=$1 nl un low list
  nl=$(grep -c . "$f"); un=$(grep -c ' perpass=-1$' "$f")
  low=$(awk -v m="$MIN_PASS_BRANCHES" '{ for (i = 1; i <= NF; i++) if ($i ~ /^perpass=/) { v = substr($i, 9) + 0; if (v >= 0 && v < m) n++ } } END { print n + 0 }' "$f")
  list=$(sed -n 's/.* perpass=\([-0-9]*\)$/\1/p' "$f" | tr '\n' ',' | sed 's/,$//')
  if [ "$nl" = 0 ]; then echo "no-loops"
  elif [ "$un" != 0 ]; then echo "unrecognised loops=$nl perpass=$list"
  elif [ "$low" != 0 ]; then echo "below loops=$nl min=$MIN_PASS_BRANCHES perpass=$list"
  else echo "ok loops=$nl min=$MIN_PASS_BRANCHES perpass=$list"; fi; }
# loop_gate: after BCMP-P2-EXACT, on the stripped P2 object: a hoisted __g1_start load, a missing loop, a counter form the parser does not recognise or fewer than MIN_PASS_BRANCHES conditional
# branches per pass in any inlined copy is a STOP (V_CMP takes the code, so no link and no probe follow); nothing else is read from the disassembly
loop_gate() { local C=$R/cmp/up os pst
  dis_order $C/dis-P2.txt > $C/dis-loops-P2.raw
  grep '^S ' $C/dis-loops-P2.raw | cut -c3- > $C/dis-loops-P2.txt
  grep '^D ' $C/dis-loops-P2.raw | cut -c3- > $C/loop-bodies-P2.txt
  os=$(order_status $C/dis-loops-P2.txt); pst=$(loop_perpass_status $C/dis-loops-P2.txt)
  vr X-p2-loops "order=[$os] perpass=[$pst]" INFO
  case $os in
    ok*) ;;
    hoisted*) V_CMP=DISASM-ORDER-HOISTED; return;;
    no-loops) V_CMP=LOOP-NO-LOOPS; return;;
    *) V_CMP=DISASM-ORDER-PATTERN; return;;
  esac
  case $pst in
    ok*) ;;
    below*) V_CMP=LOOP-BELOW;;
    unrecognised*) V_CMP=LOOP-UNRECOGNISED;;
    *) V_CMP=LOOP-NO-LOOPS;;
  esac; }

# ---- X: U (X1b's libc.a, read only) against P2 (this job's), members with debug info stripped. x1b's cmp_up_phase with U taken from X1b's sysroot.
# Expected [I]: exactly one differing member, pthread_cond_wait.o, i.e. BCMP-P2-EXACT. Anything else is X1E-STOP-BCMP/<code>, no link and no probe.
cmp_p2_phase() { local C=$R/cmp/up a m n names Ulib=$X1B/U/sysroot/usr/lib/libc.a Plib=$R/P2/sysroot/usr/lib/libc.a
  V_CMP=NOT-RUN
  ok_to_start || { V_CMP="NOT-RUN/$REFUSED"; vr X-p2-start refused "$V_CMP"; return; }
  mkdir -p $C/U $C/P2
  (cd $C/U && nice -n 10 taskset -c $CORES ar x $Ulib) || { V_CMP=BCMP-INCONCLUSIVE/AR; vr X-p2-ar U $V_CMP; return; }
  (cd $C/P2 && nice -n 10 taskset -c $CORES ar x $Plib) || { V_CMP=BCMP-INCONCLUSIVE/AR; vr X-p2-ar P2 $V_CMP; return; }
  ar t $Ulib | sort > $C/members-U.txt; ar t $Plib | sort > $C/members-P2.txt
  vr X-p2-members "U=$(grep -c . $C/members-U.txt) P2=$(grep -c . $C/members-P2.txt) dup-U=$(uniq -d $C/members-U.txt | grep -c .) dup-P2=$(uniq -d $C/members-P2.txt | grep -c .)" INFO
  if [ "$(uniq -d $C/members-U.txt | grep -c .)" != 0 ] || [ "$(uniq -d $C/members-P2.txt | grep -c .)" != 0 ] || ! cmp -s $C/members-U.txt $C/members-P2.txt; then V_CMP=BCMP-INCONCLUSIVE/MEMBERSET; vr X-p2-set "$(comm -3 $C/members-U.txt $C/members-P2.txt | head -5 | tr '\n' ' ')" $V_CMP; return; fi
  for a in U P2; do (cd $C/$a && find . -name '*.o' -exec nice -n 10 taskset -c $CORES strip --strip-debug {} +) || { V_CMP=BCMP-INCONCLUSIVE/STRIP; vr X-p2-strip $a $V_CMP; return; }; done
  : > $C/differ.txt; while read -r m; do cmp -s "$C/U/$m" "$C/P2/$m" || echo "$m" >> $C/differ.txt; done < $C/members-U.txt
  n=$(grep -c . $C/differ.txt); names=$(tr '\n' ' ' < $C/differ.txt | sed 's/ $//')
  vr X-p2-differ "n=$n [$names]" INFO
  if [ "$n" = 0 ]; then V_CMP=BCMP-P2-NONE
  elif [ "$names" = pthread_cond_wait.o ]; then V_CMP=BCMP-P2-EXACT
  elif grep -qx pthread_cond_wait.o $C/differ.txt; then V_CMP=BCMP-P2-EXTRA
  else V_CMP=BCMP-P2-WRONG; fi
  vr X-p2 "$V_CMP" $V_CMP
  for a in U P2; do objdump -d --no-show-raw-insn $C/$a/pthread_cond_wait.o > $C/dis-$a.txt 2>&1; done
  vr X-p2-disasm "pause-insns U=$(grep -cE '[[:space:]]pause' $C/dis-U.txt) P2=$(grep -cE '[[:space:]]pause' $C/dis-P2.txt); syscall-insns U=$(grep -c 'syscall' $C/dis-U.txt) P2=$(grep -c 'syscall' $C/dis-P2.txt); imm-0x9c40-or-0x9c3f U=$(grep -cE '0x9c40|0x9c3f' $C/dis-U.txt) P2=$(grep -cE '0x9c40|0x9c3f' $C/dis-P2.txt)" INFO
  [ "$V_CMP" = BCMP-P2-EXACT ] && loop_gate
  vr X-p2-gate "$V_CMP" "$V_CMP"
  diff -rq $X1B/U/sysroot/usr/include $R/P2/sysroot/usr/include | sed -e "s|$X1B/||g" -e "s|$R/||g" > $C/hdr-diff.txt; vr X-p2-hdr "$(grep -c . $C/hdr-diff.txt) lines" INFO; }

# ---- BR: arm branch and include isolation for the P2 sysroot (x1c's branch_gate, one arm)
branch_gate() { local a=P2 S br nv want rc
  local -a fl
  S=$R/$a/sysroot; fl=("--sysroot=$S" "-B$S/usr/lib/" -std=gnu11 -O2 -g -Wall -Wextra -Werror -pthread -Isrc "-D$(armdef $a)")
  step br-E-$a 120 $CLK gcc "${fl[@]}" -E src/cl_condraw.c; rc=$?
  if [ $rc = 99 ]; then V_BR="NOT-RUN/$REFUSED"; return 1; fi
  step br-M-$a 120 $CLK gcc "${fl[@]}" -M src/cl_condraw.c; [ $? = 99 ] && { V_BR="NOT-RUN/$REFUSED"; return 1; }
  nv=$(inc_viol br-M-$a "$S"); br=$(branch_of $L/br-E-$a.log)
  want="arm_u=1 arm_c=0 value64=1"
  vr BRANCH-$a "$br include_violations=$nv" INFO
  if [ $rc != 0 ] || [ "$nv" != 0 ] || [ "$br" != "$want" ]; then V_BR="FAIL-BRANCH:$a"; return 1; fi
  V_BR=BR-OK; vr BRANCH "arm branch active and every include from the sysroot, arm P2 (U layout)" BR-OK; return 0; }
# sdk_phase: the C SDK built against the P2 headers (X1b ck_arm step 1); the archive is not a glibc component and is pinned by its sha256 for the record
sdk_phase() { local B=$CK/b-P2 rc
  mkdir -p $B
  step sdk-P2 300 $CLK make -s sdk "CC=$(cc_of P2)" "BUILD=$B"; rc=$?
  if [ $rc = 99 ]; then V_SDK="NOT-RUN/$REFUSED"; return 1; fi
  if [ $rc != 0 ] || [ ! -f $B/sdk/libhairspring.a ]; then V_SDK="FAIL-SDK:build"; vr SDK "make sdk rc=$rc" "$V_SDK"; return 1; fi
  vr SDKSHA-P2 "$(sha_of $B/sdk/libhairspring.a)" INFO
  V_SDK=SDK-OK; vr SDK "libhairspring.a built against the P2 headers" SDK-OK; return 0; }

# ---- B1/B2: build the two binaries (link only), x1c's pass 1 and pass 2 for two names
# def_check NAME : the -DCONDLOCK_* tokens of the make -n command line are exactly {arm define, the name's define (none for the counted build)}. 0 ok, 1 bad, 99 refused. (x1c's, with the counted build's empty define)
def_check() { local n=$1 B rc got want
  spec_of "$n" || return 1
  B=$CK/b-$SP_ARM
  step "def-$n" 60 $CLK env "CFLAGS=-D$(armdef $SP_ARM)" make -n "$B/$SP_TARGET" "CC=$(cc_of $SP_ARM)" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && return 99
  [ $rc = 0 ] || return 1
  got=$(tr -s ' \\' '\n' < "$L/def-$n.log" | grep -E '^-DCONDLOCK_' | LC_ALL=C sort | tr '\n' ' ')
  if [ "$SP_TOK" = - ]; then want=$(printf '%s\n' "-D$(armdef $SP_ARM)" | LC_ALL=C sort | tr '\n' ' ')
  else want=$(printf '%s\n%s\n' "-D$(armdef $SP_ARM)" "$SP_TOK" | LC_ALL=C sort | tr '\n' ' '); fi
  vr "DEF-$n" "defines=[$got]" "$([ "$got" = "$want" ] && echo DEF-OK || echo "DEF-BAD want=[$want]")"
  [ "$got" = "$want" ]; }
# x1c: build_one
# build_one PASS NAME OUTDIR : one make; the product is moved to OUTDIR/NAME. 0 ok, 1 failed, 99 refused.
build_one() { local ps=$1 n=$2 out=$3 B rc
  spec_of "$n" || return 1
  B=$CK/b-$SP_ARM
  step "b$ps-$n" 120 $CLK env "CFLAGS=-D$(armdef $SP_ARM)" make -s "$B/$SP_TARGET" "CC=$(cc_of $SP_ARM)" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && return 99
  { [ $rc = 0 ] && [ -x "$B/$SP_TARGET" ]; } || return 1
  mv "$B/$SP_TARGET" "$out/$n"; }
# bin_iso NAME : X1b's CISO gates for one pass 1 binary (x1c's, with the sysroot of this job)
bin_iso() { local n=$1 b=$PASS1/$1 lg=$L/b1-$1.log S B v c sn v2=0 v3 why=
  spec_of "$n"; S=$R/$SP_ARM/sysroot; B=$CK/b-$SP_ARM
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
# pass1: x1c's, over this job's two names
pass1() { local n rc h
  : > $R/pass1.sha
  for n in $(all_names); do
    def_check "$n"; rc=$?
    if [ $rc = 99 ]; then V_B1="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then V_B1="FAIL-DEF:$n"; return 1; fi
    build_one 1 "$n" "$PASS1"; rc=$?
    if [ $rc = 99 ]; then V_B1="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then vr "B1-$n" "make rc!=0 or no product" BUILD-FAIL; V_B1="FAIL-BUILD:$n"; return 1; fi
    bin_iso "$n" || { V_B1="FAIL-ISO:$n"; return 1; }
    h=$(sha_of "$PASS1/$n"); echo "$h  $n" >> $R/pass1.sha; vr "B1-$n" "$h" BUILT
  done
  if [ "$(awk '{print $1}' $R/pass1.sha | LC_ALL=C sort | uniq -d | grep -c .)" != 0 ]; then vr B1-uniq "two binaries share a sha256" DUPLICATE; V_B1="FAIL-UNIQ:duplicate-sha256"; return 1; fi
  V_B1=B1-OK; vr B1 "3 binaries built, isolated, distinct" B1-OK; return 0; }
# x1c: cmp_bins
# cmp_bins A B : "identical", or what differs: bytes, sizes, and whether the files are equal once debug info and the build-id are stripped
cmp_bins() { local a=$1 b=$2 n s=differ t=$R/tmp
  if cmp -s "$a" "$b"; then echo identical; return 0; fi
  n=$(cmp -l "$a" "$b" 2> /dev/null | wc -l | tr -d ' ')
  rm -f "$t/cb.a" "$t/cb.b"
  strip --strip-debug -R .note.gnu.build-id -o "$t/cb.a" "$a" 2> /dev/null; strip --strip-debug -R .note.gnu.build-id -o "$t/cb.b" "$b" 2> /dev/null
  if [ -s "$t/cb.a" ] && cmp -s "$t/cb.a" "$t/cb.b"; then s=equal; fi
  echo "differ bytes=$n sizes=$(wc -c < "$a" | tr -d ' ')/$(wc -c < "$b" | tr -d ' ') debug-and-buildid-stripped=$s"; }
# pass2: x1c's, over this job's two names, after the SDK archive has been rebuilt at the same path too (the pass 1 SDK directory is moved aside, never deleted)
pass2() { local n rc h1 h2 nd=0 B=$CK/b-P2 s1 s2
  : > $R/pass2.sha
  s1=$(sha_of $B/sdk/libhairspring.a)
  mv $B/sdk $R/sdk-pass1 || { vr B2-sdk "the pass 1 SDK directory could not be moved aside" BUILD-FAIL; V_B2=FAIL-BUILD2:sdk; return 1; }
  step sdk2-P2 300 $CLK make -s sdk "CC=$(cc_of P2)" "BUILD=$B"; rc=$?
  if [ $rc = 99 ]; then V_B2="NOT-RUN/$REFUSED"; return 1; fi
  if [ $rc != 0 ] || [ ! -f $B/sdk/libhairspring.a ]; then vr B2-sdk "make sdk rc=$rc" BUILD-FAIL; V_B2=FAIL-BUILD2:sdk; return 1; fi
  s2=$(sha_of $B/sdk/libhairspring.a)
  vr B2-sdk "pass1=$s1 pass2=$s2" "$([ "$s1" = "$s2" ] && echo IDENTICAL || echo INFO-DIFFERS)"
  for n in $(all_names); do
    build_one 2 "$n" "$PASS2"; rc=$?
    if [ $rc = 99 ]; then V_B2="NOT-RUN/$REFUSED"; return 1; fi
    if [ $rc != 0 ]; then vr "B2-$n" "make rc!=0 or no product" BUILD-FAIL; V_B2="FAIL-BUILD2:$n"; return 1; fi
    h1=$(sha_of "$PASS1/$n"); h2=$(sha_of "$PASS2/$n"); echo "$h2  $n" >> $R/pass2.sha
    if [ "$h1" = "$h2" ]; then vr "B2-$n" "$h2" IDENTICAL
    else nd=$((nd + 1)); vr "B2-$n" "pass1=$h1 pass2=$h2 $(cmp_bins "$PASS1/$n" "$PASS2/$n")" DIFFERS; fi
  done
  if [ $nd = 0 ]; then V_B2=REPRO; else V_B2="NONREPRO-$nd"; fi
  vr B2 "$nd of 3 differ between pass 1 and pass 2" "$V_B2"; return 0; }
# pin_phase: x1c's, over this job's two names; the table GLG would copy from, and the reused inputs
pin_phase() { local n
  : > $R/pin-table.tsv
  for n in $(all_names); do
    spec_of "$n"
    if ! { cp "$PASS1/$n" "$PIN/$n" && chmod 555 "$PIN/$n" && cmp -s "$PASS1/$n" "$PIN/$n"; }; then V_PIN="FAIL-PIN:$n"; vr PIN "copy of $n failed" "$V_PIN"; return 1; fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$n" "$(sha_of "$PIN/$n")" "$(wc -c < "$PIN/$n" | tr -d ' ')" "$SP_ARM" "$SP_TOK" >> $R/pin-table.tsv
  done
  { printf 'libc.a-P2\t%s\n' "$(sha_of $R/P2/sysroot/usr/lib/libc.a)"; printf 'libhairspring.a-P2\t%s\n' "$(sha_of $R/sdk-pass1/libhairspring.a)"
    printf 'libc.a-U\t%s\nx1b-condlock-C\t%s\n' "$(pin_of LIBC U)" "$(pin_of BIN C)"
    printf 'glibc-P2.tar\t%s\npositive-control-p2.patch\t%s\n' "$PIN_P2TAR" "$PIN_PATCH"
    printf 'condlock.tar\t%s\nclassify.sh\t%s\nhermit\t%s\ngcc\t%s\n' "$PIN_CLTAR" "$PIN_CLS" "$HERMIT_SHA" "$(gcc -dumpfullversion 2> /dev/null)"; } > $R/pin-inputs.tsv
  V_PIN=PINNED; vr PIN "$(grep -c . $R/pin-table.tsv) rows; table sha256 $(sha_of $R/pin-table.tsv)" PINNED; return 0; }

# ---- S: native smoke on the pinned pass 1 files. Classes come from the repo's classify.sh (pinned by sha256). Nothing here says whether the property site fires.
# x1c: line_ok
line_ok() { # file planted(0|1) : exactly one non-empty line, in the report format, with ` applied=` iff planted
  local f=$1 pl=$2 first
  [ "$(grep -c . "$f")" = 1 ] || return 1
  first=$(grep -m 1 . "$f")
  [[ $first =~ $RE_OUT ]] || return 1
  if [ "$pl" = 1 ]; then [[ $first == *" applied="* ]]; else [[ $first != *" applied="* ]]; fi; }
# x1c: smoke_class
smoke_class() { # id rc : sets SM_CLS SM_AP SM_SUB from classify.sh
  local cl; cl=$(sh $CLK/scripts/classify.sh "$2" $L/$1.out $L/$1.err 2>&1 | head -n 1)
  SM_CLS=${cl%% *}; SM_CLS=${SM_CLS#class=}; SM_SUB=-; SM_AP=-
  if [[ $cl =~ subtype=([A-Za-z+-]+) ]]; then SM_SUB=${BASH_REMATCH[1]}; fi
  if [[ $cl =~ applied=([01-]) ]]; then SM_AP=${BASH_REMATCH[1]}; fi; }
# t2_expect NAME RC CLASS APPLIED : prints OK or the reason. Both binaries are unplanted, so the only expectation is the allowed exit set and the three rated classes; no expectation says hit or no hit.
t2_expect() { local n=$1 rc=$2 cls=$3
  spec_of "$n" || { echo unknown-name; return; }
  case $rc in 0|3|4) ;; *) echo "rc=$rc"; return;; esac
  case $cls in NOHIT|HIT|STALL) echo OK;; *) echo "class=$cls";; esac; }
smoke1() { local n=$1 id=sm1-$1 rc why=
  spec_of "$n"
  rc=$(runone "$id" 60 "$PASS1/$n" $SM1_ARGS)
  case $rc in 0|3|4) ;; *) why="$why rc=$rc";; esac
  line_ok "$L/$id.out" 0 || why="$why report-line"
  vr "SM1-$n" "rc=$rc" "$([ -z "$why" ] && echo OK || echo "BAD:${why# }")"
  [ -z "$why" ]; }
smoke2() { local n=$1 rep=$2 id=sm2-$1-$2 rc why='' r
  spec_of "$n"
  rc=$(runone "$id" 60 "$PASS1/$n" $SM2_ARGS)
  smoke_class "$id" "$rc"
  line_ok "$L/$id.out" 0 || why="$why report-line"
  r=$(t2_expect "$n" "$rc" "$SM_CLS"); [ "$r" = OK ] || why="$why $r"
  vr "SM2-$n-$rep" "rc=$rc class=$SM_CLS applied=$SM_AP subtype=$SM_SUB" "$([ -z "$why" ] && echo OK || echo "BAD:${why# }")"
  [ -z "$why" ]; }
smoke_phase() { local n rep bad nbad=0 rf='' nn
  nn=$(smoke_names | wc -l | tr -d ' ')
  for n in $(smoke_names); do
    ok_to_start || { rf=$REFUSED; break; }
    bad=0; smoke1 "$n" || bad=1
    for rep in $(seq $SM2_REPS); do smoke2 "$n" "$rep" || bad=1; done
    if [ $bad = 0 ]; then vr "SM-$n" "tier 1 and $SM2_REPS tier 2 runs as expected" SMOKE-OK; else nbad=$((nbad + 1)); vr "SM-$n" "a run was not as expected" SMOKE-BAD; fi
  done
  if [ -n "$rf" ]; then V_SMOKE="NOT-RUN/$rf"; vr SMOKE "refused before $n" "$V_SMOKE"; return 1; fi
  if [ $nbad = 0 ]; then V_SMOKE=SMOKE-OK; else V_SMOKE="FAIL-SMOKE:$nbad"; fi
  vr SMOKE "$nbad of $nn binaries had a run that was not as expected" "$V_SMOKE"; [ $nbad = 0 ]; }

# ---- H: the probe. hermit_run is byte-identical to x1c and x1d.
# x1d: hermit_run
hermit_run() { local id=$1 bin=$2 seed=$3 d=$H/runs/$1 t0 rc; shift 3
  mkdir -p $d/tmp $H/hout || { echo 99; return; }
  chmod 1777 $d/tmp $H/hout || { echo 99; return; }
  t0=$(date +%s%N)
  ( ulimit -c 0; cd $d && sudo -n timeout -k 10 120 env -i PATH=$CPATH HOME=/root LANG=C nice -n 10 taskset -c $HCORE $HERMIT run --epoch=2026-01-01T00:00:00Z --base-env=minimal \
      --chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot --sched-seed=$seed --tmp=$d/tmp --summary-json=$H/hout/$id.summary.json -- $H/$bin "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; rc=$?
  echo $(( ($(date +%s%N) - t0) / 1000000 )) > $L/$id.wallms; echo $rc; }
# x1d: px_class
# px_class ID RC : like x1c's smoke_class, and the property flag (prop_false of the repo's classify.sh)
px_class() { local cl; cl=$(sh $CLK/scripts/classify.sh "$2" $L/$1.out $L/$1.err 2>&1 | head -n 1)
  PX_CLS=${cl%% *}; PX_CLS=${PX_CLS#class=}; PX_SUB=-; PX_PF=-
  if [[ $cl =~ subtype=([A-Za-z+-]+) ]]; then PX_SUB=${BASH_REMATCH[1]}; fi
  if [[ $cl =~ prop_false=([01]) ]]; then PX_PF=${BASH_REMATCH[1]}; fi; }
px_first() { case $1 in 125) echo 319891;; 250) echo 319921;; *) echo 0;; esac; }   # first seed of a cell
# px_cell R : PX_N serial Hermit runs of the probed binary at -r R; rows only, nothing graded; three consecutive failed runs (a run not rated HIT, NOHIT or STALL, or ended by the cap or by Hermit) end the whole probe
px_cell() { local r=$1 i seed id rc ticks dump hnote fail
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
    hnote=$(grep -m 1 -E 'SKID_OVERSHOOT|panicked at' $L/$id.err | cut -c1-300)
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$r" "$seed" "$rc" "$PX_CLS" "$PX_PF" "$PX_SUB" "$(cat $L/$id.wallms 2> /dev/null)" "$(jnum $H/hout/$id.summary.json virttime_elapsed)" "$(jnum $H/hout/$id.summary.json sched_turns)" "${ticks:-NA}" "${dump:-none}" "${hnote:-none}" >> $R/px.tsv
    vr "H-$id" "rc=$rc class=$PX_CLS prop_false=$PX_PF subtype=$PX_SUB wall_ms=$(cat $L/$id.wallms 2> /dev/null) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns) ticks=${ticks:-NA} dump=[${dump:-none}] hermit_note=[${hnote:-none}]" INFO
    fail=0; case $rc in 124|137|125|101) fail=1;; esac
    case $PX_CLS in NOHIT|HIT|STALL) ;; *) fail=1;; esac
    if [ $fail = 1 ]; then PX_CONSEC=$((PX_CONSEC + 1)); else PX_CONSEC=0; fi
    [ $PX_CONSEC -ge 3 ] && PX_STOP=failures
  done; }
px_summary() { local r=$1
  vr "PXSUM-$r" "$(awk -F'\t' -v r="$r" '$1==r{n++; if($5==1)k++; c[$4]++; w=$7+0; if(n==1||w<mn)mn=w; if(w>mx)mx=w; if(w>6000)o++; t+=$10; if($12!="none")s++}
    END{printf "runs=%d prop_false=%d wall_ms_min=%d wall_ms_max=%d wall_over_6000ms=%d ticks_sum=%d hermit_note_rows=%d classes=[", n, k, mn, mx, o, t, s; for(x in c) printf "%s:%d,", x, c[x]; printf "]"}' $R/px.tsv)" INFO; }
px_phase() { local r rc
  [ "$(sha_of $HERMIT)" = $HERMIT_SHA ] || { V_PX=PX-NOT-RUN/PIN; vr PX "Hermit sha256 differs" $V_PX; return 0; }
  ok_to_start || { V_PX="PX-NOT-RUN/$REFUSED"; vr PX refused "$V_PX"; return 0; }
  # shellcheck disable=SC2024  # deliberate: the redirect is done by this (uid 1000) shell; sudo only runs /bin/true
  sudo -n /bin/true > $L/px-sudo.log 2>&1 || { V_PX=PX-NOT-RUN/SUDO; vr PX "sudo -n /bin/true failed" $V_PX; return 0; }
  mkdir -p $H/runs $H/hout || { V_PX=PX-NOT-RUN/NODIR; vr PX fail $V_PX; return 0; }
  chmod 1777 $H/hout
  cp $PIN/$PX_BIN $H/$PX_BIN 2> /dev/null
  cmp -s $PIN/$PX_BIN $H/$PX_BIN || { V_PX=PX-NOT-RUN/COPY; vr PX "$PX_BIN missing or the copy differs" $V_PX; return 0; }
  rc=$(runone px-exec 60 $H/$PX_BIN $SM1_ARGS)
  case $rc in 0|3|4) ;; *) V_PX=PX-NOT-RUN/NOEXEC; vr PX "rc=$rc" $V_PX; return 0;; esac
  : > $R/px.tsv; PX_STOP=0; PX_CONSEC=0; PX_RF=
  for r in $PX_CELLS; do px_cell $r; px_summary $r; done
  if [ $PX_STOP = failures ]; then V_PX=PX-STOPPED-FAILURES
  elif [ $PX_STOP = refused ]; then V_PX="PX-PARTIAL/$PX_RF"
  else V_PX=PX-RAN; fi
  vr PX "$(grep -c . $R/px.tsv) of $((PX_N * 2)) runs recorded" "$V_PX"; return 0; }

# ---- O: the overall code (mechanical; no count, class, wall time or tick of the probe enters it)
failcode() { local k=${1#FAIL-}; echo "X1E-FAIL-${k%%:*}/${k#*:}"; }
overall() { local v
  case $V_TOOLS in TOOLS-BUILT) ;; *) echo X1E-INCONCLUSIVE/TOOLS; return;; esac
  case $V_SRC in SRC-OK) ;; NOT-RUN/*) echo "X1E-INCONCLUSIVE/${V_SRC#NOT-RUN/}"; return;; *) echo "X1E-FAIL-SOURCES/$V_SRC"; return;; esac
  case $V_BUILD in BUILT) ;; NOT-RUN/*) echo "X1E-INCONCLUSIVE/${V_BUILD#NOT-RUN/}"; return;; BUILD-TIMEOUT) echo X1E-INCONCLUSIVE/BUILD-TIMEOUT; return;; *) echo "X1E-FAIL-BUILD/$V_BUILD"; return;; esac
  case $V_CMP in BCMP-P2-EXACT) ;; BCMP-INCONCLUSIVE*|NOT-RUN*) echo X1E-INCONCLUSIVE/CMP; return;; *) echo "X1E-STOP-BCMP/$V_CMP"; return;; esac
  for v in "$V_BR" "$V_SDK" "$V_B1" "$V_B2" "$V_PIN" "$V_SMOKE"; do
    case $v in
      BR-OK|SDK-OK|B1-OK|REPRO|NONREPRO-*|PINNED|SMOKE-OK) ;;
      NOT-RUN/*) echo "X1E-INCONCLUSIVE/${v#NOT-RUN/}"; return;;
      FAIL-*) failcode "$v"; return;;
      *) echo "X1E-UNKNOWN/$v"; return;;
    esac
  done
  case $V_PX in
    PX-RAN) echo X1E-PASS;;
    PX-STOPPED-FAILURES) echo X1E-HERMIT-STOPPED;;
    PX-PARTIAL/*) echo "X1E-INCONCLUSIVE/${V_PX#PX-PARTIAL/}";;
    PX-NOT-RUN/*) echo "X1E-INCONCLUSIVE/${V_PX#PX-NOT-RUN/}";;
    *) echo "X1E-UNKNOWN/$V_PX";;
  esac; }
res_suffix() { local s=
  case $V_B2 in NONREPRO-*) s="$s+$V_B2";; esac
  [ -z "$KHDRDIFF" ] || s="$s+KHDR-DIFFERS-X1B"
  echo "$s"; }
side_effects() { local s=
  cmp -s $R/ldcache-before.txt $R/ldcache-post.txt || s="$s+LDCACHE-CHANGED"
  [ -s $R/leftover.txt ] && s="$s+LEFTOVER"
  cmp -s $R/tmp-before.txt $R/tmp-post.txt || s="$s+TMP-LISTING-DIFFERS"
  cmp -s $R/shm-before.txt $R/shm-post.txt || s="$s+SHM-LISTING-DIFFERS"
  cmp -s $R/crash-before.txt $R/crash-post.txt || s="$s+VAR-CRASH-DIFFERS"
  echo "$s$INCH$(res_suffix)"; }
overall_line() { printf 'OVERALL\t%s%s\ttools=%s src=%s build=%s cmp=%s branch=%s sdk=%s b1=%s b2=%s pin=%s smoke=%s px=%s\n' "$1" "$2" "$V_TOOLS" "$V_SRC" "$V_BUILD" "$V_CMP" "$V_BR" "$V_SDK" "$V_B1" "$V_B2" "$V_PIN" "$V_SMOKE" "$V_PX"; }

# ---- test seam: the Mac stub tests source this file with X1E_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${X1E_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "X1E_TEST is set in a real run: refused" >&2; exit 5; fi
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
vr X-cpu "$(grep -m 1 'model name' /proc/cpuinfo | cut -c1-120)" INFO

# ---- main
gates
ck_extract
setup_copy
tools_phase; V_TOOLS=$LASTV   # a tools failure is never a glibc failure
if [ "$V_TOOLS" = TOOLS-BUILT ]; then
  if src_phase && khdr_phase; then V_SRC=SRC-OK; else V_SRC=$LASTV; fi
  if [ "$V_SRC" = SRC-OK ]; then
    khdr_cmp
    build_arm P2; V_BUILD=$LASTV
    STEPCLASS=A
    if [ "$V_BUILD" = BUILT ]; then
      cmp_p2_phase
      if [ "$V_CMP" = BCMP-P2-EXACT ] && branch_gate && sdk_phase && pass1; then
        if pass2 && pin_phase; then
          if smoke_phase; then px_phase; else V_PX=PX-NOT-RUN/SMOKE; fi
        fi
      fi
    fi
  fi
fi
finish_up
overall_line "$(overall)" "$(side_effects)" >> $R/cases.tsv
echo DONE-OVERALL >> $R/steps.tsv
