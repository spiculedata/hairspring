#!/usr/bin/env bash
# usage: setsid nohup timeout -k 30 4500 bash $R/bin/x1b.sh $R 1,2,3 <stamp> > $R/run.out 2>&1 < /dev/null &
# X1b = G0 rev 8's g0.sh with the positive-control tree P and the condlock part added (section 15 of the spec has the diff against g0.sh).
set -u
R=$1; CORES=$2; STAMP=$3; L=$R/logs; BIN=$R/bin
H=/dev/shm/x1b-glibc-$STAMP; HCORE=${CORES%%,*}
CK=$R/ck; CLK=$CK/examples/glibc-condlock
HERMIT=/path/to/hermit   # EDIT: the pinned hermit binary (sha256 must equal HERMIT_SHA)
HERMIT_SHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
CPATH=/usr/bin:/bin   # PATH of the plain runs and of the root Hermit wrapper: never changed
STEPPATH=$CPATH       # PATH of the logged steps; phase T puts $R/tools/bin first (uid 1000 only)
T0=$(date +%s); BDL=$((T0 + 45 * 60)); NEWBY_B=$((T0 + 40 * 60)); NEWBY=$((T0 + 68 * 60))
RCAP_KB=12582912; HCAP_KB=102400; FREE_MIN_KB=20971520
NPLAIN=50; SEEDS="319001 319002 319003"; REPS=2
WANT_LINE() { echo "condsmoke ok libc=2.40 hdr=2.40 signalled=1 woke=1"; }
SYS_LINE="condsmoke ok libc=2.39 hdr=2.39 signalled=1 woke=1"
STEPCLASS=A
# X1b pins and pre-registered thresholds (spec sections 3 and 9; none is tuned after data)
U_FILE_SHA=55ce6de06fc59642cae8a82c63c683fb39856fdcf5a6cbc5b3940ba8cabc7e25   # nptl/pthread_cond_wait.c in U
P_FILE_SHA=52071add40648057720f675cc9bba9aede6c7a31fc6e6e241117502d22f6211d   # the same file in P (U plus the one hunk)
MARKER=G0P-POSITIVE-CONTROL-HUNK
LIBREF_U=939cc8ec3cadfa5feb81b30f467a1e81dadb7af60297c7277355f62f1ab157f4   # libc.a sha256 in the G0p container: recorded next to the box values, never graded
LIBREF_C=8f0a4fb96af6b0a3bcf9d6cad0bb1c9aadbdf7a733e3653f221fa5b82c0a7016
LIBREF_P=b85b130d8efb031f1411231e71dd016af2a3b25503d553d2671a2653fddb5002
NCL=50                                        # condlock plain runs per arm (arguments: cl_run)
TSEED_SLEEP=319701; TSEED_PP=319702           # T-probe seeds (design 7.2)
FF_WALL_MS=10000; FF_VIRT_NS=60000000000      # tprobe i and iii: wall under 10 s and at least 60 s of virtual time
PFF_B_NS=7000000                              # P against U (condsmoke, medians of 6): at least 7 ms more virtual time
PFF_C_VIRT_NS=5000000000; PFF_C_RATIO=2       # P against U, tprobe ii 1000, same seed: at least 5 s more virtual time, and 2 x (extra wall ms) x 1e6 below the extra virtual ns
declare -A VX CLFIRE
HREADY=0; INCH=
note() { printf '%s\t%s\n' "$(date +%s)" "$*" >> $R/events.tsv; }
LASTV=; BW=0
vr() { LASTV=$3; printf 'K\t%s\t%s\t%s\n' "$1" "$2" "$3" >> $R/cases.tsv; note "K $1 $3"; }   # id, raw value, verdict (LASTV = the verdict just written)
vx() { vr "$1" "$2" "$3"; VX[$1]=$3; }   # vr, and keep the verdict under its id for overall()
REFUSED=
# ok_to_start: returns 0, or returns 1 with REFUSED = STOP|HALT|BUDGET|PAUSED|DISK|PROC. A refusal is never a failure of the thing that was about to run.
ok_to_start() { local now; now=$(date +%s); REFUSED=
  [ -e $R/STOP ] && { REFUSED=STOP; note STOPPED; return 1; }
  [ -e $R/HALT ] && { REFUSED=HALT; note "HALTED: $(cat $R/HALT)"; return 1; }
  [ "$now" -ge $NEWBY ] && { REFUSED=BUDGET; note "BUDGET: no new step after 68 min"; return 1; }
  [ "$STEPCLASS" = B ] && [ "$now" -ge $NEWBY_B ] && { REFUSED=BUDGET; note "BUDGET: no new build step after 40 min"; return 1; }
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && { REFUSED=PAUSED; note "PAUSED: heavy job started"; return 1; }
  [ "$(du -sk $R | cut -f1)" -gt $RCAP_KB ] && { REFUSED=DISK; note "DISK: $R over 12 GB cap"; return 1; }
  [ -d $H ] && [ "$(du -sk $H 2> /dev/null | cut -f1)" -gt $HCAP_KB ] && { REFUSED=DISK; note "DISK: $H over 100 MB cap"; return 1; }
  [ "$(df -P -k "$HOME" | awk 'NR==2{print $4}')" -lt $FREE_MIN_KB ] && { REFUSED=DISK; note "DISK: less than 20 GB free"; return 1; }
  for _ in 1 2 3 4 5; do { procs_with_dir $R; procs_with_dir $H; } > $R/procs-now.txt; [ -s $R/procs-now.txt ] || break; sleep 1; done
  [ -s $R/procs-now.txt ] && { REFUSED=PROC; note "REFUSED: a process has the work dir in its arguments: $(head -n 1 $R/procs-now.txt | cut -c1-200)"; return 1; }
  return 0; }
# bv id rc failcode [timeoutcode] : verdict of a step from its rc. 99 = refused to start (NOT-RUN/<reason>), 124/137 = timeout, else failcode.
bv() { local id=$1 rc=$2 fail=$3 tmo=${4:-BUILD-TIMEOUT}
  if [ "$rc" = 99 ]; then vr $id "rc=99 refused" "NOT-RUN/${REFUSED:-UNKNOWN}"
  elif [ "$rc" = 124 ] || [ "$rc" = 137 ]; then vr $id rc=$rc "$tmo"
  else vr $id rc=$rc "$fail"; fi; }
snapshot() { { date -u +%FT%TZ; cat /proc/loadavg; ps -eo pid,ppid,pgid,sid,ni,psr,pcpu,etimes,stat,comm --sort=-pcpu | head -30; free -m; cat /proc/mdstat
    df -P -k "$HOME" /dev/shm; sha256sum /etc/ld.so.cache
    echo "cpu1 governor: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_governor 2>&1) cur_khz: $(cat /sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq 2>&1)"
    pgrep -a -x 'hermit|qemu-system-x86|cargo|rustc|make|cc1|cc1plus|gcc|ld|condsmoke-U|condsmoke-C|condsmoke-P|condsmoke-SYS|tprobe-U|tprobe-C|tprobe-P|condlock'
    ls -la --time-style=full-iso /var/crash
    f() { awk '/^cpu[0-9]/{print $1,$5+$6,$2+$3+$4+$5+$6+$7+$8+$9}' /proc/stat; }; f > $R/s1.$1; sleep 5; f > $R/s2.$1
    paste $R/s1.$1 $R/s2.$1 | awk '{printf "%s %.1f%%\n",$1,100*(1-($5-$2)/($6-$3))}'; } > $R/snap-$1.txt 2>&1
  ls -1 /tmp | sort > $R/tmp-$1.txt; ls -la --time-style=full-iso /var/crash > $R/crash-$1.txt 2>&1; sha256sum /etc/ld.so.cache > $R/ldcache-$1.txt
  ls -1 /dev/shm | sort > $R/shm-$1.txt; ls -lan --time-style=full-iso /tmp /dev/shm > $R/own-$1.txt 2>&1   # own-: numeric owners, to list the root-owned entries that are left in place
  { du -sk $R; [ -d $H ] && du -sk $H; } > $R/du-$1.txt 2>&1; }   # root-written entries under $H may be unreadable: partial du, errors in the file
leftovers() { local p e; pgrep -s $SID > $R/sess.tmp   # no command substitution around pgrep; the driver ($$) and the timeout that owns the session ($PPID) are excluded
  while read -r p; do [ "$p" = $$ ] || [ "$p" = $PPID ] || echo "session $p $(tr '\0' ' ' < /proc/$p/cmdline 2> /dev/null)"; done < $R/sess.tmp
  for p in /proc/[0-9]*; do e=$(readlink $p/exe 2> /dev/null); case $e in $R/*|$H/*) echo "exe-scan ${p#/proc/} $e";; esac; done
  ps -eo pid=,user=,sid=,args= | RP="$H/" awk 'index($0,ENVIRON["RP"]) { print "root-scan " $0 }'; }   # ubuntu cannot readlink a root process's exe: any user's process whose command line names $H/
# every process of ANY user, in ANY session, whose arguments contain "$1/". Excluded by pid: the driver's ancestor chain ANC (guard.sh of GON/GOG, F0-e rev 3) and the driver's own direct children (the pipeline's ps and awk).
procs_with_dir() { ps -eo pid=,ppid=,user=,sid=,args= | RP="$1/" awk -v me="$$" -v anc="$ANC" 'index(anc, " " $1 " ") == 0 && $2!=me && index($0,ENVIRON["RP"])'; }
step() { # id timeout cwd cmd... : one logged step, clean env, pinned cores, nice 10
  local id=$1 to=$2 cwd=$3 t0 rc; shift 3
  ok_to_start || return 99
  t0=$(date +%s.%N)
  ( ulimit -c 0; cd "$cwd" && env -i PATH=$STEPPATH HOME=$R LANG=C LC_ALL=C TMPDIR=$R/tmp nice -n 10 taskset -c $CORES timeout -k 30 "$to" "$@" ) > $L/$id.log 2>&1
  rc=$?
  printf '%s\t%s\t%s\t%s\n' "$id" "$rc" "$(awk -v a=$t0 -v b="$(date +%s.%N)" 'BEGIN{printf "%.1f",b-a}')" "$*" >> $R/steps.tsv; return $rc; }
bto() { echo $(( BDL - $(date +%s) )); }
# end_checks: the inputs and the Hermit pin are hashed again at the end (they were hashed on both machines before launch); the helpers are recorded
end_checks() { INCH=
  (cd $R && sha256sum -c tars.sha > $R/end-tars.txt 2>&1 && sha256sum -c tools.sha > $R/end-tools.txt 2>&1) || INCH="$INCH+INPUT-CHANGED"
  { [ -x $HERMIT ] && [ "$(sha256sum $HERMIT | cut -d' ' -f1)" = $HERMIT_SHA ]; } || INCH="$INCH+HERMIT-CHANGED"
  (cd $BIN && sha256sum x1b.sh tools.sh condsmoke.c) > $R/end-helpers.txt 2>&1; }
# rootowned: everything under $H that is not owned by ubuntu (Hermit runs as root), listed and LEFT IN PLACE; errors from unreadable root-created directories go to the same file
rootowned() { find $H ! -user ubuntu -printf '%u %m %p\n' > $R/rootowned.txt 2>&1; vr Z-rootowned "$(grep -c . $R/rootowned.txt) lines (root-owned entries and permission errors, left in place)" INFO; }
finish_up() { end_checks; snapshot post; leftovers > $R/leftover.txt; rootowned; note "LEFTOVER=$(wc -l < $R/leftover.txt) DONE"; echo DONE >> $R/steps.tsv; }
# on_signal: TERM, INT or HUP (the 4500 s cap, a Mac-side kill of the ssh session is not one): HALT, the post snapshot and the leftover scan, then exit 6 (GON/GOG guard.sh convention)
on_signal() { trap '' TERM INT HUP; echo SIGNAL > $R/HALT; note "SIGNAL: the driver was told to stop; HALT written"; finish_up
  printf 'OVERALL\tINCONCLUSIVE/SIGNAL\n' >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 6; }
# small helpers
jnum() { sed -n "s/^ *\"$2\": *\([0-9][0-9]*\).*/\1/p" "$1" 2> /dev/null | head -n 1; }   # jnum summary.json key : the first integer after "key":
median_of() { local -a a; mapfile -t a < <(sort -n); local n=${#a[@]}   # numbers on stdin, one per line; prints the median (the lower middle average for an even count), or nothing
  [ "$n" -gt 0 ] || return 0
  if [ $((n % 2)) = 1 ]; then echo "${a[$((n / 2))]}"; else echo $(( (a[n / 2 - 1] + a[n / 2]) / 2 )); fi; }
inc_viol() { # log sysroot : absolute include paths in a gcc -M log that are neither in the sysroot, nor gcc's own, nor the condlock tree
  tr -s ' \\' '\n' < $L/$1.log | grep '^/' | grep -v -e "^$2/" -e "^$(realpath "$2")/" -e '^/usr/lib/gcc/x86_64-linux-gnu/13/' -e "^$CK/" | wc -l | tr -d ' '; }
armdef() { case $1 in C) echo CONDLOCK_ARM_C;; *) echo CONDLOCK_ARM_U;; esac; }   # P has the U layout (G0p: the two header files are byte-equal)
branch_of() { # preprocessed cl_condraw.c : which layout branch is live (the silent ARM_NONE fallback shows as 0 and 0)
  echo "arm_u=$(grep -c 'raw.g_refs\[0\] = c->__data.__g_refs\[0\]' $1) arm_c=$(grep -c 'raw.arm = CL_ARM_C;' $1) value64=$(grep -c 'raw.wseq = c->__data.__wseq.__value64' $1)"; }
cl_suffix() { local a s=   # overall-code suffixes from the condlock plain cells: a hit or stall in U or C, a vacuous cell
  for a in U C; do [ "${CLFIRE[$a]:-0}" != 0 ] && s="$s+CL-FIRED-$a"; done
  for a in U C P; do [ "${VX[CPLAIN-$a]:-}" = CPLAIN-PASS-VACUOUS ] && s="$s+VACUOUS-$a"; done
  echo "$s"; }

# ---- T: private bison (and the GNU m4 it needs at run time), built from pinned GNU tarballs as uid 1000, no sudo, no host change, prefix $R/tools
# tools.sha (sha256 of bison-3.8.2.tar.gz and m4-1.4.19.tar.gz, verified on the Mac against the GPG signatures) is checked here; $R/tools/bin then goes first in STEPPATH for every later step.
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

# ---- A: sources (tars were verified on both machines before launch; verified again here). X1b: three arms, and the checks on the one-hunk patch of P
src_phase() {
  local a f n want cnt rc uf pf mp mu mc
  step A-tarsum 300 $R sha256sum -c tars.sha; rc=$?; [ $rc = 0 ] || { bv A-tarsum $rc SRC-FAIL SRC-FAIL; return 1; }
  for a in U C P; do mkdir -p $R/$a; step A-x-$a 900 $R tar -xf glibc-$a.tar -C $R/$a; rc=$?; [ $rc = 0 ] || { bv A-x-$a $rc SRC-FAIL SRC-FAIL; return 1; }
    n=$(find $R/$a/src \( -type f -o -type l \) | wc -l); want=$(awk -v a=$a '$1==a{print $2}' $R/counts.txt)
    vr A-count-$a "$n want=$want" "$([ "$n" = "$want" ] && echo OK || echo SRC-FAIL)"; [ "$n" = "$want" ] || return 1; done
  for a in U C P; do for f in nptl/pthread_cond_broadcast.c nptl/pthread_cond_common.c nptl/pthread_cond_signal.c nptl/pthread_cond_wait.c nptl/tst-cond22.c sysdeps/nptl/bits/thread-shared-types.h sysdeps/nptl/pthread.h; do
      (cd $R/$a/src && sha256sum $f); done; done > $R/srcfiles.txt
  diff -rq $R/U/src $R/C/src | sed "s|$R/||g" > $R/srcdiff.txt
  cnt=$(grep -c '^Files ' $R/srcdiff.txt); vr A-srcdiff "differing=$cnt other=$(grep -vc '^Files ' $R/srcdiff.txt)" "$([ "$cnt" = 7 ] && [ "$(grep -vc '^Files ' $R/srcdiff.txt)" = 0 ] && echo OK || echo SRC-FAIL)"
  [ "$cnt" = 7 ] && [ "$(grep -vc '^Files ' $R/srcdiff.txt)" = 0 ] || return 1
  vr A-U-line "$(grep -cF 'uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);' $R/U/src/nptl/pthread_cond_wait.c)" INFO-exact-line-count-in-U
  vr A-C-line "$(grep -cF 'uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);' $R/C/src/nptl/pthread_cond_wait.c)" INFO-exact-line-count-in-C
  vr A-P-line "$(grep -cF 'uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);' $R/P/src/nptl/pthread_cond_wait.c)" INFO-exact-line-count-in-P
  # P1: U and P differ in exactly one file
  diff -rq $R/U/src $R/P/src | sed "s|$R/||g" > $R/srcdiff-UP.txt
  vr A-srcdiff-UP "$(tr '\n' ' ' < $R/srcdiff-UP.txt | cut -c1-300)" "$([ "$(cat $R/srcdiff-UP.txt)" = "Files U/src/nptl/pthread_cond_wait.c and P/src/nptl/pthread_cond_wait.c differ" ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P2: that file is the pinned one in U and in P
  uf=$(sha256sum $R/U/src/nptl/pthread_cond_wait.c | cut -d' ' -f1); pf=$(sha256sum $R/P/src/nptl/pthread_cond_wait.c | cut -d' ' -f1)
  vr A-filesha "U=$uf P=$pf" "$([ "$uf" = $U_FILE_SHA ] && [ "$pf" = $P_FILE_SHA ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P3: the marker comment is counted once in P and zero times in U and C
  mp=$(grep -cF "$MARKER" $R/P/src/nptl/pthread_cond_wait.c); mu=$(grep -cF "$MARKER" $R/U/src/nptl/pthread_cond_wait.c); mc=$(grep -cF "$MARKER" $R/C/src/nptl/pthread_cond_wait.c)
  vr A-marker "P=$mp U=$mu C=$mc" "$([ "$mp" = 1 ] && [ "$mu" = 0 ] && [ "$mc" = 0 ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  # P4: diff of U's file against P's reproduces exactly the added lines of the shipped patch (one range line, no removed line)
  diff $R/U/src/nptl/pthread_cond_wait.c $R/P/src/nptl/pthread_cond_wait.c > $R/hunk.diff
  grep '^+' $R/positive-control.patch | grep -v '^+++' | cut -c2- > $R/hunk-patch.txt
  grep '^>' $R/hunk.diff | cut -c3- > $R/hunk-diff.txt
  vr A-hunk "range: $(grep '^[0-9]' $R/hunk.diff | tr '\n' ' ') added=$(grep -c '^>' $R/hunk.diff) removed=$(grep -c '^<' $R/hunk.diff) patch-added=$(grep -c . $R/hunk-patch.txt)+blank" \
    "$(cmp -s $R/hunk-patch.txt $R/hunk-diff.txt && [ "$(grep -c '^<' $R/hunk.diff)" = 0 ] && [ "$(grep -c '^[0-9]' $R/hunk.diff)" = 1 ] && echo OK || echo SRC-FAIL)"; [ "$LASTV" = OK ] || return 1
  vr A-install_root "$(grep -c 'install_root' $R/U/src/Makeconfig)" INFO
  [ -x $R/U/src/configure ] || { vr A-configure missing SRC-FAIL; return 1; }
  return 0; }

# ---- K: kernel headers (read from the host, copied into $R so the glibc build never looks at /usr/include)
khdr_phase() { local rc
  step K-copy 300 $R bash -c "mkdir -p $R/khdr && cp -a /usr/include/linux $R/khdr/linux && cp -a /usr/include/asm-generic $R/khdr/asm-generic && cp -a /usr/include/x86_64-linux-gnu/asm $R/khdr/asm"; rc=$?
  [ $rc = 0 ] || { bv K-copy $rc KHDR-FAIL KHDR-FAIL; return 1; }
  (cd $R/khdr && find . -type f | sort | xargs sha256sum | sha256sum) > $R/khdr.manifest 2>&1
  dpkg-query -W linux-libc-dev > $R/khdr.pkg 2>&1; return 0; }

# ---- C: the condlock tree (condlock.tar: examples/glibc-condlock and sdks/c at the pinned commit; the C SDK pin is its PINS file)
ck_extract() { local rc
  mkdir -p $CK
  step C-x-ck 300 $R tar -xf condlock.tar -C $CK; rc=$?; [ $rc = 0 ] || { bv C-x-ck $rc CK-FAIL CK-FAIL; return 1; }
  vr C-pins "$(tr '\n' ' ' < $CLK/PINS | cut -c1-300)" INFO
  vr C-files "$(find $CK -type f | wc -l | tr -d ' ') files" INFO
  if grep -qx 'SDK_COMMIT=a4131e49680fd80edf66d560a5cbf4be54de0cd7' $CLK/PINS && grep -qx 'SDK_TREE=f5d13d7773d6211e9ebb2283f774bc4b00cd6013' $CLK/PINS \
    && [ -f $CLK/src/cl_condraw.c ] && [ -f $CLK/src/tprobe.c ] && [ -f $CLK/Makefile ] && [ -f $CLK/scripts/classify.sh ] && [ -f $CK/sdks/c/Makefile ]; then vr C-layout ok OK; return 0; fi
  vr C-layout missing CK-FAIL; return 1; }

# ---- B: build one arm (configure, make, install), all in its own out-of-tree directory
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
# libc_ref: the box libc.a sha256 next to the G0p container value, information only (the build paths differ)
libc_ref() { local a h r
  for a in U C P; do h=$(sha256sum $R/$a/sysroot/usr/lib/libc.a | cut -d' ' -f1)
    case $a in U) r=$LIBREF_U;; C) r=$LIBREF_C;; P) r=$LIBREF_P;; esac
    vr X-libref-$a "box=$h container=$r equal=$([ "$h" = "$r" ] && echo yes || echo no)" INFO; done; }

# ---- L: link the smoke program statically against the arm's own sysroot, then prove isolation
iso_trace() { # arm log [tag] : prints the number of violations; raw offenders go to $L/iso-<tag>.txt (tag defaults to the arm)
  local a=$1 log=$2 S pat t=${3:-$1}; S=$R/$a/sysroot; pat='(crt1|crti|crtn|Scrt1|rcrt1|gcrt1)\.o|lib(c|m|pthread|dl|rt|util|resolv|anl|mvec|c_nonshared)\.(a|so)|ld-linux'
  grep -v '^attempt to open .* failed' $log > $L/iso-$t.tr
  { grep -E "$pat" $L/iso-$t.tr | grep -vF "$S/"; grep -E '/(usr/)?lib(64)?/x86_64-linux-gnu/' $L/iso-$t.tr; } > $L/iso-$t.txt
  grep -c . $L/iso-$t.txt; }
libc_hits() { grep -cF -e "$2/usr/lib/libc.a(" -e "($2/usr/lib/libc.a)" "$1"; }   # both trace forms: path(member) and (path)member
link_arm() { local a=$1 S O GI GIF CB CE LGD v k rc
  S=$R/$a/sysroot; O=$R/$a/smoke
  mkdir -p $O; GI=$(gcc -print-file-name=include); GIF=$(gcc -print-file-name=include-fixed); CB=$(gcc -print-file-name=crtbeginT.o); CE=$(gcc -print-file-name=crtend.o); LGD=$(dirname "$(gcc -print-libgcc-file-name)")
  echo "$GI $GIF $CB $CE $LGD" > $O/gcc-paths.txt
  step LK1-$a 300 $O gcc --sysroot=$S -O2 -g -pthread -static -B$S/usr/lib/ -L$S/usr/lib -Wl,-t,-t -o $O/condsmoke-L1 $BIN/condsmoke.c; rc=$?
  [ $rc = 99 ] && { vr L-$a "refused" "NOT-RUN/$REFUSED"; return 1; }
  k=1; v=$(iso_trace $a $L/LK1-$a.log)
  if [ ! -x $O/condsmoke-L1 ] || [ "$v" != 0 ] || [ "$(libc_hits $L/LK1-$a.log $S)" = 0 ]; then
    step LK2-$a 300 $O gcc -O2 -g -pthread -static -nostdinc -isystem "$GI" -isystem "$GIF" -isystem $S/usr/include -nostartfiles -nodefaultlibs -o $O/condsmoke-L2 \
      $S/usr/lib/crt1.o $S/usr/lib/crti.o "$CB" $BIN/condsmoke.c -L$S/usr/lib -L"$LGD" -Wl,-t,-t -Wl,--start-group -lc -lgcc -lgcc_eh -Wl,--end-group "$CE" $S/usr/lib/crtn.o; rc=$?
    [ $rc = 99 ] && { vr L-$a "refused" "NOT-RUN/$REFUSED"; return 1; }
    k=2; v=$(iso_trace $a $L/LK2-$a.log); fi
  if [ -x $O/condsmoke-L$k ] && [ "$v" = 0 ] && [ "$(libc_hits $L/LK$k-$a.log $S)" != 0 ]; then
    cp $O/condsmoke-L$k $O/condsmoke-$a; vr L-$a "L$k trace_violations=0" LINK-OK-L$k; return 0; fi
  vr L-$a "L$k trace_violations=$v" LINK-FAIL; return 1; }
iso_arm() { local a=$1 b why n
  b=$R/$a/smoke/condsmoke-$a; why=
  readelf -lW $b > $L/iso-$a.lw 2>&1; readelf -dW $b > $L/iso-$a.dw 2>&1; readelf -hW $b > $L/iso-$a.hw 2>&1; timeout 10 ldd $b > $L/iso-$a.ldd 2>&1
  [ "$(grep -c INTERP $L/iso-$a.lw)" = 0 ] || why="$why interp"
  grep -q 'There is no dynamic section' $L/iso-$a.dw || why="$why dynamic-section"
  grep -q 'not a dynamic executable' $L/iso-$a.ldd || why="$why ldd"
  grep -q 'Advanced Micro Devices X86-64' $L/iso-$a.hw || why="$why machine"
  strings -a $b > $L/iso-$a.strings; n=$(grep -c 'Ubuntu GLIBC' $L/iso-$a.strings); [ "$n" = 0 ] || why="$why ubuntu-glibc-string"
  nm $b > $L/iso-$a.nm 2>&1; vr I-nm-$a "pthread_cond_wait_common symbols: $(grep -c 'pthread_cond_wait_common' $L/iso-$a.nm)" INFO
  sha256sum $b >> $R/binaries.sha
  vr I-$a "${why:-none}" "$([ -z "$why" ] && echo ISOLATED || echo NOT-ISOLATED:$why)"; [ -z "$why" ]; }

# ---- P: plain runs; runone writes stdout/stderr files and prints rc
runone() { local id=$1 tmo=$2; shift 2
  ( ulimit -c 0; cd $R && env -i PATH=$CPATH HOME=$R LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$tmo" "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; echo $?; }
plain_arm() { local a=$1 b=$2 want=$3 i rc ok=0 rf=
  for i in $(seq $NPLAIN); do [ $((i % 10)) = 1 ] && { ok_to_start || { rf=$REFUSED; break; }; }; rc=$(runone plain-$a-$i 10 $b)
    [ "$rc" = 0 ] && [ "$(cat $L/plain-$a-$i.out)" = "$want" ] && ok=$((ok + 1)); done
  if [ -n "$rf" ]; then vr P-$a "$ok/$NPLAIN done before refusal" "NOT-RUN/$rf"; return 1; fi
  vr P-$a "$ok/$NPLAIN" "$([ "$ok" = $NPLAIN ] && echo PLAIN-PASS || echo PLAIN-FAIL-$ok/$NPLAIN)"; [ "$ok" = $NPLAIN ]; }

# ---- W: the condlock workload (the C SDK, tprobe and condlock per arm), linked static against the arm's own sysroot; its isolation gate; its plain cell
cl_run() { # id arm : one plain run of the arm's condlock with the cell's arguments (spec section 3); prints the exit status
  local id=$1 a=$2 b=$CK/b-$2/condlock
  case $a in P) runone $id 60 $b -n 8 -r 60 -tick-ms 50 -q 10;; *) runone $id 60 $b -n 8 -r 5000 -tick-ms 50 -q 10;; esac; }
ck_arm() { local a=$1 S=$R/$1/sysroot B=$CK/b-$1 cc rc nv br want
  local -a fl
  mkdir -p $B
  cc="gcc --sysroot=$S -B$S/usr/lib/ -static -L$S/usr/lib -Wl,-t,-t"
  fl=("--sysroot=$S" "-B$S/usr/lib/" -std=gnu11 -O2 -g -Wall -Wextra -Werror -pthread -Isrc "-D$(armdef $a)")
  step sdk-$a 300 $CLK make -s sdk "CC=$cc" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && { vx CLINK-$a refused "NOT-RUN/$REFUSED"; return 1; }
  if [ $rc != 0 ] || [ ! -f $B/sdk/libhairspring.a ]; then vx CLINK-$a "sdk rc=$rc" CLINK-FAIL-sdk; return 1; fi
  vr SDKSHA-$a "$(sha256sum $B/sdk/libhairspring.a | cut -d' ' -f1)" INFO
  step tprobe-$a 120 $CLK make -s tprobe "CC=$cc" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && { vx CLINK-$a refused "NOT-RUN/$REFUSED"; return 1; }
  if [ $rc != 0 ] || [ ! -x $B/tprobe ]; then vx CLINK-$a "tprobe rc=$rc" CLINK-FAIL-tprobe; return 1; fi
  step link-$a 300 $CLK env "CFLAGS=-D$(armdef $a)" make -s condlock "CC=$cc" "BUILD=$B"; rc=$?
  [ $rc = 99 ] && { vx CLINK-$a refused "NOT-RUN/$REFUSED"; return 1; }
  if [ $rc != 0 ] || [ ! -x $B/condlock ]; then vx CLINK-$a "condlock rc=$rc" CLINK-FAIL-link; return 1; fi
  step ck-E-$a 120 $CLK gcc "${fl[@]}" -E src/cl_condraw.c; rc=$?
  [ $rc = 99 ] && { vx CLINK-$a refused "NOT-RUN/$REFUSED"; return 1; }
  step ck-M-$a 120 $CLK gcc "${fl[@]}" -M src/cl_condraw.c; rc=$?
  [ $rc = 99 ] && { vx CLINK-$a refused "NOT-RUN/$REFUSED"; return 1; }
  nv=$(inc_viol ck-M-$a "$S"); br=$(branch_of $L/ck-E-$a.log)
  case $a in C) want="arm_u=0 arm_c=1 value64=1";; *) want="arm_u=1 arm_c=0 value64=1";; esac
  vr CK-BRANCH-$a "$br include_violations=$nv" INFO
  if [ "$nv" != 0 ]; then vx CLINK-$a "include_violations=$nv" CLINK-FAIL-include; return 1; fi
  if [ "$br" != "$want" ]; then vx CLINK-$a "$br want=$want" CLINK-FAIL-branch; return 1; fi
  vx CLINK-$a "sdk, tprobe and condlock built (rc 0); arm branch active; every include from the sysroot" CLINK-OK; return 0; }
ciso_arm() { local a=$1 S=$R/$1/sysroot B=$CK/b-$1 lg=$L/link-$1.log b v n sn v2 v3 rc line why=
  b=$B/condlock
  v=$(iso_trace $a $lg cl-$a); [ "$v" = 0 ] || why="$why trace-violations=$v"
  n=$(libc_hits $lg $S); [ "$n" != 0 ] || why="$why no-libc-member-line"
  sn=$(grep -cF "$B/sdk/libhairspring.a" $lg); [ "$sn" != 0 ] || why="$why no-sdk-member-line"
  # the SDK archive is not a glibc component: its own trace lines pass the glibc rule with 0 violations, and the rule still rejects a system glibc line
  grep -F "$B/sdk/libhairspring.a" $lg > $L/sdk-lines-$a.txt
  v2=$(iso_trace $a $L/sdk-lines-$a.txt sdkctl-$a); [ "$v2" = 0 ] || why="$why sdk-counted-as-glibc"
  printf '%s\n' '(/usr/lib/x86_64-linux-gnu/libc.a)printf.o' > $L/negctl-$a.in
  v3=$(iso_trace $a $L/negctl-$a.in negctl-$a); [ "$v3" != 0 ] || why="$why rule-insensitive"
  readelf -lW $b > $L/ciso-$a.lw 2>&1; readelf -dW $b > $L/ciso-$a.dw 2>&1; readelf -hW $b > $L/ciso-$a.hw 2>&1; timeout 10 ldd $b > $L/ciso-$a.ldd 2>&1
  [ "$(grep -c INTERP $L/ciso-$a.lw)" = 0 ] || why="$why interp"
  grep -q 'There is no dynamic section' $L/ciso-$a.dw || why="$why dynamic-section"
  grep -q 'not a dynamic executable' $L/ciso-$a.ldd || why="$why ldd"
  grep -q 'Advanced Micro Devices X86-64' $L/ciso-$a.hw || why="$why machine"
  strings -a $b > $L/ciso-$a.strings; [ "$(grep -c 'Ubuntu GLIBC' $L/ciso-$a.strings)" = 0 ] || why="$why ubuntu-glibc-string"
  rc=$(cl_run ciso-run-$a $a); line=$(head -n 1 $L/ciso-run-$a.out | cut -d' ' -f1-3)
  [ "$line" = "condlock libc=2.40 hdr=2.40" ] || why="$why runtime-line:$line"
  sha256sum $b >> $R/binaries.sha
  vx CISO-$a "violations=$v libc_member_lines=$n sdk_lines=$sn sdk_control=$v2 negative_control=$v3 run_rc=$rc ${why:-none}" "$([ -z "$why" ] && echo CISO-OK || echo NOT-CISO:$why)"; [ -z "$why" ]; }
clplain_arm() { local a=$1 i rc cl cls line t ms b1 sp v tsum=0 mx=0 b1sum=0 spsum=0 ok=0 hit=0 stall=0 bad=0 lbad=0 n=0 rf=
  : > $R/clp-$a.tsv   # one row per run: run, exit status, class, ticks, maxstag, blocked1, spanned (the watchdog evidence of every run)
  for i in $(seq $NCL); do
    [ $((i % 10)) = 1 ] && { ok_to_start || { rf=$REFUSED; break; }; }
    rc=$(cl_run clp-$a-$i $a); n=$((n + 1))
    cl=$(sh $CLK/scripts/classify.sh "$rc" $L/clp-$a-$i.out $L/clp-$a-$i.err 2>&1 | head -n 1); cls=${cl%% *}; cls=${cls#class=}
    line=$(head -n 1 $L/clp-$a-$i.out | cut -d' ' -f1-3); [ "$line" = "condlock libc=2.40 hdr=2.40" ] || lbad=$((lbad + 1))
    t=$(sed -n 's/.* ticks= *\([0-9][0-9]*\).*/\1/p' $L/clp-$a-$i.out | head -n 1); tsum=$((tsum + ${t:-0}))
    ms=$(sed -n 's/.* maxstag= *\([0-9][0-9]*\).*/\1/p' $L/clp-$a-$i.out | head -n 1); [ "${ms:-0}" -gt $mx ] && mx=$ms
    b1=$(sed -n 's/.* blocked1= *\([0-9][0-9]*\).*/\1/p' $L/clp-$a-$i.out | head -n 1); b1sum=$((b1sum + ${b1:-0}))
    sp=$(sed -n 's/.* spanned= *\([0-9][0-9]*\).*/\1/p' $L/clp-$a-$i.out | head -n 1); spsum=$((spsum + ${sp:-0}))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$i" "$rc" "$cls" "${t:-NA}" "${ms:-NA}" "${b1:-NA}" "${sp:-NA}" >> $R/clp-$a.tsv
    case $cls in NOHIT) ok=$((ok + 1));; HIT) hit=$((hit + 1));; STALL) stall=$((stall + 1));; *) bad=$((bad + 1));; esac; done
  CLFIRE[$a]=$((hit + stall))
  if [ -n "$rf" ]; then vx CPLAIN-$a "$n/$NCL done before refusal" "NOT-RUN/$rf"; return 1; fi
  if [ $bad != 0 ] || [ $lbad != 0 ]; then v="CPLAIN-FAIL-$((bad + lbad))/$NCL"
  elif [ $tsum -lt $((3 * NCL)) ]; then v=CPLAIN-PASS-VACUOUS
  else v=CPLAIN-PASS; fi
  vx CPLAIN-$a "n=$n nohit=$ok hit=$hit stall=$stall bad=$bad libc_line_bad=$lbad ticks_sum=$tsum mean_ticks_x10=$((tsum * 10 / n)) maxstag=$mx blocked1_sum=$b1sum spanned_sum=$spsum" "$v"; [ "${v%%-FAIL*}" = "$v" ]; }

# ---- H: the same binaries under the pinned Hermit (fresh --tmp per run; binaries copied to $H because Hermit cannot read the home directory)
# Hermit runs as root through `sudo -n` (it could not run as uid 1000 on the test machine; every earlier Hermit run there was root via sudo -n). The only sudo uses in this
# script are this command, the `sudo -n /bin/true` probe in hermit_phase, and (by hand, after the run) the cleanup `sudo -n kill` of listed pids. Every path Hermit
# writes (--tmp, --summary-json) is in an ubuntu-created mode-1777 directory (D1 rows s1/n0 passed there; a 755 ubuntu-owned summary dir panicked at tool_global.rs:1619).
# stdout/stderr are redirected outside sudo, so the logs stay ubuntu-owned. Prints the rc (99 = setup problem, not a Hermit result). X1b: <bin> is a file name under $H, the guest's arguments follow the seed, and the wall time (ms, sudo to exit) goes to $L/<id>.wallms.
hermit_run() { local id=$1 bin=$2 seed=$3 d=$H/runs/$1 t0 rc; shift 3
  mkdir -p $d/tmp $H/hout || { echo 99; return; }
  chmod 1777 $d/tmp $H/hout || { echo 99; return; }
  t0=$(date +%s%N)
  ( ulimit -c 0; cd $d && sudo -n timeout -k 10 120 env -i PATH=$CPATH HOME=/root LANG=C nice -n 10 taskset -c $HCORE $HERMIT run --base-env=minimal --epoch=2026-01-01T00:00:00Z \
      --chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot --sched-seed=$seed --tmp=$d/tmp --summary-json=$H/hout/$id.summary.json -- $H/$bin "$@" ) > $L/$id.out 2> $L/$id.err < /dev/null; rc=$?
  echo $(( ($(date +%s%N) - t0) / 1000000 )) > $L/$id.wallms; echo $rc; }
# hermit_class id rc want : OK | SUDO (sudo itself refused: empty stdout and a `sudo:` line; not a Hermit result) | SETUP (rc 99) | FAIL. OK needs rc 0 AND the expected line AND a clean stderr.
hermit_class() { local id=$1 rc=$2 want=$3
  if [ "$rc" = 99 ]; then echo SETUP
  elif [ "$rc" != 0 ] && [ ! -s $L/$id.out ] && grep -q -E '^sudo:' $L/$id.err; then echo SUDO
  elif [ "$rc" = 0 ] && [ "$(cat $L/$id.out)" = "$want" ] && [ "$(grep -cE 'Deadlock detected|panicked|SKID_OVERSHOOT|HERMIT_INTERNAL_FAILURE' $L/$id.err)" = 0 ]; then echo OK
  else echo FAIL; fi; }
hermit_phase() { local a seed rep id rc v cls nok nfail nnr vfail consec=0 rf=
  local -A OKN=([U]=0 [C]=0 [P]=0) FAILN=([U]=0 [C]=0 [P]=0) NRN=([U]=0 [C]=0 [P]=0)
  [ -x $HERMIT ] && [ "$(sha256sum $HERMIT | cut -d' ' -f1)" = $HERMIT_SHA ] || { vr H-pin "$(sha256sum $HERMIT 2>&1 | cut -c1-64)" HERMIT-NOT-RUN/PIN; return; }
  ok_to_start || { vr H-start "refused" "HERMIT-NOT-RUN/$REFUSED"; return; }
  # shellcheck disable=SC2024  # deliberate: the redirect must be done by this (uid 1000) shell, so the log stays ubuntu-owned; sudo only runs /bin/true
  sudo -n /bin/true > $L/h-sudo.log 2>&1 || { vr H-sudo "sudo -n /bin/true failed" HERMIT-NOT-RUN/SUDO; return; }   # NOT-RUN, never a failure
  mkdir -p $H/runs || { vr H-dir fail HERMIT-NOT-RUN/NODIR; return; }
  cp $R/U/smoke/condsmoke-U $R/C/smoke/condsmoke-C $R/P/smoke/condsmoke-P $R/SYS/condsmoke-SYS $H/ 2> /dev/null
  for a in U C P SYS; do [ -x $H/condsmoke-$a ] || { vr H-copy-$a missing HERMIT-NOT-RUN/COPY; return; }; done
  (cd $H && sha256sum condsmoke-U condsmoke-C condsmoke-P condsmoke-SYS) > $R/h-binaries.sha; (cd $R && sha256sum U/smoke/condsmoke-U C/smoke/condsmoke-C P/smoke/condsmoke-P SYS/condsmoke-SYS) > $R/h-src-binaries.sha
  [ "$(awk '{print $1}' $R/h-binaries.sha)" = "$(awk '{print $1}' $R/h-src-binaries.sha)" ] || { vr H-copy-sha differ HERMIT-NOT-RUN/COPY; return; }
  rc=$(runone h-exec 10 $H/condsmoke-U); [ "$rc" = 0 ] || { vr H-exec "rc=$rc" HERMIT-NOT-RUN/NOEXEC; return; }
  HREADY=1
  for rep in 1 2; do id=h-SYS-319001-$rep
    if ! ok_to_start; then vr H-$id refused "NOT-RUN/$REFUSED"; continue; fi
    rc=$(hermit_run $id condsmoke-SYS 319001); cls=$(hermit_class $id "$rc" "$SYS_LINE")
    case $cls in OK) vr H-$id "rc=$rc" CONTROL-OK;; SUDO|SETUP) rf=$cls; vr H-$id "rc=$rc" "NOT-RUN/$cls";; *) vr H-$id "rc=$rc" CONTROL-FAIL;; esac; done
  for a in U C P; do for seed in $SEEDS; do for rep in $(seq $REPS); do
    [ $consec -ge 3 ] && continue
    id=h-$a-$seed-$rep
    if ! ok_to_start; then rf=$REFUSED; NRN[$a]=$((NRN[$a] + 1)); vr H-$id refused "NOT-RUN/$rf"; continue; fi   # a refusal is not a Hermit failure
    if [ "$rf" = SUDO ]; then NRN[$a]=$((NRN[$a] + 1)); vr H-$id skipped "NOT-RUN/SUDO"; continue; fi
    rc=$(hermit_run $id condsmoke-$a $seed); cls=$(hermit_class $id "$rc" "$(WANT_LINE)")
    if [ "$cls" = SUDO ] || [ "$cls" = SETUP ]; then rf=$cls; NRN[$a]=$((NRN[$a] + 1)); vr H-$id "rc=$rc" "NOT-RUN/$cls"; continue; fi   # sudo/setup problem: not a Hermit failure
    if [ "$cls" = OK ]; then consec=0; OKN[$a]=$((OKN[$a] + 1)); vr H-$id "rc=$rc out=$(sha256sum < $L/$id.out | cut -c1-16) summary=$(sha256sum $H/hout/$id.summary.json 2> /dev/null | cut -c1-16) wall_ms=$(cat $L/$id.wallms) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns)" RUN-OK
    else consec=$((consec + 1)); FAILN[$a]=$((FAILN[$a] + 1)); vr H-$id "rc=$rc" RUN-FAIL; fi; done; done; done
  # per arm: PASS = 6/6 OK; a real failed run = FAIL-k/6; otherwise (some runs refused, none failed) = NOT-RUN/<reason>
  for a in U C P; do
    nok=${OKN[$a]}; nfail=${FAILN[$a]}; nnr=${NRN[$a]}; vfail="HERMIT-FAIL-$nok/6"
    if [ "$nok" = 6 ]; then v="HERMIT-PASS"; elif [ "$nfail" != 0 ] || { [ "$nnr" = 0 ] && [ $consec -ge 3 ]; }; then v=$vfail; elif [ "$nnr" != 0 ]; then v="HERMIT-NOT-RUN/$rf"; else v=$vfail; fi
    case $a in U) V_HU=$v;; C) V_HC=$v;; P) V_HP=$v;; esac; done
  [ $consec -ge 3 ] && vr H-abort "3 consecutive failures; remaining runs not started" HERMIT-ABORTED
  vr H-U "${OKN[U]}/6 ok, ${FAILN[U]} failed, ${NRN[U]} refused" $V_HU; vr H-C "${OKN[C]}/6 ok, ${FAILN[C]} failed, ${NRN[C]} refused" $V_HC; vr H-P "${OKN[P]}/6 ok, ${FAILN[P]} failed, ${NRN[P]} refused" $V_HP; }
# tp_run id bin seed want args... : one Hermit run of a tprobe case; sets TP_CLS (OK|FAIL|SUDO|SETUP|REFUSED), TP_RC, TP_WALL (ms), TP_VIRT (ns), TP_TURNS
tp_run() { local id=$1 bin=$2 seed=$3 want=$4; shift 4
  TP_CLS=; TP_RC=; TP_WALL=; TP_VIRT=; TP_TURNS=
  if ! ok_to_start; then TP_CLS=REFUSED; return; fi
  TP_RC=$(hermit_run $id $bin $seed "$@"); TP_CLS=$(hermit_class $id "$TP_RC" "$want")
  TP_WALL=$(cat $L/$id.wallms 2> /dev/null); TP_VIRT=$(jnum $H/hout/$id.summary.json virttime_elapsed); TP_TURNS=$(jnum $H/hout/$id.summary.json sched_turns); }
# tprobe_phase: cases i and iii (fast-forward; a failure is a STOP) and ii (recorded) for every arm. Needs the Hermit smoke to have reached its run steps (HREADY).
tprobe_phase() { local a c id want w stop=0 nr=0
  if [ "$HREADY" != 1 ]; then VX[TFF]=NOT-RUN/HERMIT; vr H-tff "the Hermit smoke did not reach its run steps" "${VX[TFF]}"; return; fi
  for a in U C P; do
    cp $CK/b-$a/tprobe $H/tprobe-$a 2> /dev/null
    if [ ! -x $H/tprobe-$a ] || ! cmp -s $CK/b-$a/tprobe $H/tprobe-$a; then VX[TFF]=NOT-RUN/BIN; vr H-tff "tprobe-$a missing or the copy differs" "${VX[TFF]}"; return; fi; done
  for a in U C P; do for c in i iii; do
      id=tp-$c-$a; want="tprobe $c ok slept=60 woke=1"
      tp_run $id tprobe-$a $TSEED_SLEEP "$want" $c 60
      case $TP_CLS in
        OK) if [ "${TP_WALL:-$FF_WALL_MS}" -lt $FF_WALL_MS ] && [ "${TP_VIRT:-0}" -ge $FF_VIRT_NS ]; then w=FF-OK; else w=FF-FAIL; fi;;
        REFUSED) w="NOT-RUN/$REFUSED"; nr=1;;
        SUDO|SETUP) w="NOT-RUN/$TP_CLS"; nr=1;;
        *) w=FF-FAIL;;
      esac
      [ "$w" = FF-FAIL ] && stop=1
      vr H-$id "rc=$TP_RC wall_ms=$TP_WALL virt_ns=$TP_VIRT turns=$TP_TURNS" "$w"
    done; done
  if [ $stop = 1 ]; then VX[TFF]=TFF-STOP; elif [ $nr = 1 ]; then VX[TFF]=TFF-NOT-RUN; else VX[TFF]=TFF-OK; fi
  vr H-tff "cases i and iii on U, C and P: wall under $FF_WALL_MS ms and virtual time at least $FF_VIRT_NS ns" "${VX[TFF]}"
  [ "${VX[TFF]}" = TFF-STOP ] && return 0   # a STOP: no further Hermit run
  for a in U C P; do
    id=tp-ii-$a; tp_run $id tprobe-$a $TSEED_PP "tprobe ii ok pingpongs=1000" ii 1000
    case $TP_CLS in OK) w=REC-OK;; REFUSED) w="NOT-RUN/$REFUSED";; SUDO|SETUP) w="NOT-RUN/$TP_CLS";; *) w=REC-FAIL;; esac
    VX[TPII-$a]=$w
    vr H-$id "rc=$TP_RC wall_ms=$TP_WALL virt_ns=$TP_VIRT turns=$TP_TURNS" "$w"
  done; }
# pff_eval: P's fast-forward set (spec 9.4). (b) the P smoke has at least PFF_B_NS more virtual time than U's (medians of the 6 runs each); (c) tprobe ii 1000 against U's at the same seed: P's extra virtual time dv is at least PFF_C_VIRT_NS and its extra wall time dw is small, PFF_C_RATIO x dw < dv (Hermit's per-turn cost cancels in the differences).
pff_eval() { local s r mu mp wallp virtp wallu virtu dv dw why=
  if [ "${VX[TFF]:-}" != TFF-OK ] || [ "$V_HU" != HERMIT-PASS ] || [ "$V_HP" != HERMIT-PASS ] || [ "${VX[TPII-U]:-}" != REC-OK ]; then VX[PFF]=PFF-NOT-RUN; vr H-pff "needs TFF-OK, HERMIT-PASS for U and P, and TPII-U REC-OK" PFF-NOT-RUN; return 0; fi
  mu=$(for s in $SEEDS; do for r in $(seq $REPS); do jnum $H/hout/h-U-$s-$r.summary.json virttime_elapsed; done; done | median_of)
  mp=$(for s in $SEEDS; do for r in $(seq $REPS); do jnum $H/hout/h-P-$s-$r.summary.json virttime_elapsed; done; done | median_of)
  vr H-pff-b "median virttime_elapsed (ns) U=$mu P=$mp; P wall ms: $(for s in $SEEDS; do for r in $(seq $REPS); do cat $L/h-P-$s-$r.wallms 2> /dev/null; done; done | tr '\n' ' ')" INFO
  { [ -n "$mu" ] && [ -n "$mp" ] && [ $((mp - mu)) -ge $PFF_B_NS ]; } || why="${why}b"
  wallp=$(cat $L/tp-ii-P.wallms 2> /dev/null); virtp=$(jnum $H/hout/tp-ii-P.summary.json virttime_elapsed)
  wallu=$(cat $L/tp-ii-U.wallms 2> /dev/null); virtu=$(jnum $H/hout/tp-ii-U.summary.json virttime_elapsed)
  vr H-pff-c "tprobe ii 1000, same seed: P wall_ms=$wallp virt_ns=$virtp; U wall_ms=$wallu virt_ns=$virtu" INFO
  if [ "${VX[TPII-P]:-}" = REC-OK ] && [ -n "$wallp" ] && [ -n "$virtp" ] && [ -n "$wallu" ] && [ -n "$virtu" ]; then
    dv=$((virtp - virtu)); dw=$((wallp - wallu)); vr H-pff-c-diff "dv_ns=$dv dw_ms=$dw" INFO
    { [ $dv -ge $PFF_C_VIRT_NS ] && [ $((PFF_C_RATIO * dw * 1000000)) -lt $dv ]; } || why="${why}c"
  else why="${why}c"; fi
  if [ -z "$why" ]; then VX[PFF]=PFF-OK; else VX[PFF]="PFF-FAIL-$why"; fi
  vr H-pff "b: P minus U median >= $PFF_B_NS ns; c: (P - U) virtual >= $PFF_C_VIRT_NS ns and $PFF_C_RATIO x (P - U) wall ms x 1e6 below it" "${VX[PFF]}"; }
# clh_phase: INFORMATIONAL ONLY, never graded and never read by overall(): one condlock run per arm under Hermit (seed 319703, -r 30 -tick-ms 100 -q 10; the 120 s cap is inside hermit_run). Recorded: rc, class (the repo's classify.sh), wall, virtual time, turns, ticks.
clh_phase() { local a id rc cl cls t
  if [ "$HREADY" != 1 ] || [ "${VX[TFF]:-}" = TFF-STOP ]; then vr H-clh "not run (Hermit setup incomplete, or TFF-STOP)" INFO; return 0; fi
  for a in U C P; do id=clh-$a
    cp $CK/b-$a/condlock $H/condlock-$a 2> /dev/null
    if [ ! -x $H/condlock-$a ] || ! cmp -s $CK/b-$a/condlock $H/condlock-$a; then vr H-$id "condlock-$a missing or the copy differs" INFO; continue; fi
    if ! ok_to_start; then vr H-$id refused "INFO-NOT-RUN/$REFUSED"; continue; fi
    rc=$(hermit_run $id condlock-$a 319703 -n 8 -r 30 -tick-ms 100 -q 10)
    cl=$(sh $CLK/scripts/classify.sh "$rc" $L/$id.out $L/$id.err 2>&1 | head -n 1); cls=${cl%% *}; cls=${cls#class=}
    t=$(sed -n 's/.* ticks= *\([0-9][0-9]*\).*/\1/p' $L/$id.out | head -n 1)
    vr H-$id "rc=$rc class=$cls wall_ms=$(cat $L/$id.wallms 2> /dev/null) virt_ns=$(jnum $H/hout/$id.summary.json virttime_elapsed) turns=$(jnum $H/hout/$id.summary.json sched_turns) ticks=${t:-NA}" INFO
  done; }

# ---- X: compare the builds (libc.a members with debug info stripped; installed tree; both reported raw)
cmp_phase() { local a m C=$R/cmp differ extras missing hdr_ok f e
  mkdir -p $C; ok_to_start || { V_CMP="NOT-RUN/$REFUSED"; vr X-start refused "$V_CMP"; return; }
  for a in U C; do mkdir -p $C/$a; (cd $C/$a && nice -n 10 taskset -c $CORES ar x $R/$a/sysroot/usr/lib/libc.a) || { V_CMP=BCMP-INCONCLUSIVE/AR; vr X-ar $a $V_CMP; return; }
    ar t $R/$a/sysroot/usr/lib/libc.a | sort > $C/members-$a.txt; done
  vr X-members "U=$(grep -c . $C/members-U.txt) C=$(grep -c . $C/members-C.txt) dup-U=$(uniq -d $C/members-U.txt | grep -c .) dup-C=$(uniq -d $C/members-C.txt | grep -c .)" INFO
  if [ "$(uniq -d $C/members-U.txt | grep -c .)" != 0 ] || [ "$(uniq -d $C/members-C.txt | grep -c .)" != 0 ] || ! cmp -s $C/members-U.txt $C/members-C.txt; then V_CMP=BCMP-INCONCLUSIVE/MEMBERSET; vr X-set "$(comm -3 $C/members-U.txt $C/members-C.txt | head -5 | tr '\n' ' ')" $V_CMP; return; fi
  for a in U C; do (cd $C/$a && find . -name '*.o' -exec nice -n 10 taskset -c $CORES strip --strip-debug {} +); done
  : > $C/differ.txt; while read -r m; do cmp -s "$C/U/$m" "$C/C/$m" || echo "$m" >> $C/differ.txt; done < $C/members-U.txt
  differ=$(grep -c . $C/differ.txt); extras=$(grep -vxE 'pthread_cond_(broadcast|signal|wait)\.o' $C/differ.txt); missing=
  for m in pthread_cond_broadcast.o pthread_cond_signal.o pthread_cond_wait.o; do grep -qx "$m" $C/differ.txt || missing="$missing $m"; done
  vr X-differ "n=$differ missing:[${missing# }] extras:[$(echo "$extras" | tr '\n' ' ')]" INFO
  if ! grep -qx 'pthread_cond_wait.o' $C/differ.txt; then V_CMP=BCMP-NONE
  elif [ -z "$extras" ] && [ -z "$missing" ]; then V_CMP=BCMP-EXACT
  elif [ -z "$extras" ]; then V_CMP="BCMP-SUBSET-missing:${missing# }"
  else hdr_ok=1; nodep=0; for e in $extras; do f=$(find $R/C/build -name "$e.d")
      if [ "$(echo "$f" | grep -c .)" != 1 ]; then nodep=1; break; fi
      grep -qE 'thread-shared-types\.h|/pthread\.h' "$f" || hdr_ok=0; done
    if [ $nodep = 1 ]; then V_CMP=BCMP-INCONCLUSIVE/DEPFILE; else V_CMP=$([ $hdr_ok = 1 ] && echo BCMP-HDR || echo BCMP-EXTRA); fi; fi
  vr X-libc "$V_CMP" $V_CMP
  diff -rq $R/U/sysroot $R/C/sysroot | sed "s|$R/||g" > $C/tree-diff.txt; vr X-tree "$(grep -c . $C/tree-diff.txt) lines" INFO
  diff -rq $R/U/sysroot/usr/include $R/C/sysroot/usr/include | sed "s|$R/||g" > $C/hdr-diff.txt; vr X-hdr "$(grep -c . $C/hdr-diff.txt) lines" INFO
  cmp -l $R/U/smoke/condsmoke-U $R/C/smoke/condsmoke-C 2> /dev/null | wc -l > $C/smoke-cmp.txt; vr X-smoke "differing bytes: $(tr -d ' ' < $C/smoke-cmp.txt)" INFO
  for a in U C; do objdump -d --no-show-raw-insn --disassemble=__pthread_cond_wait_common $R/$a/smoke/condsmoke-$a 2> /dev/null | sed -E 's/^ +[0-9a-f]+:\t//; s/0x[0-9a-f]+/ADDR/g; s/<[^>]*>//g; s/^[0-9a-f]+ //' > $C/wait-$a.dis; done
  vr X-wait-dis "U=$(grep -c . $C/wait-U.dis) lines sha $(sha256sum < $C/wait-U.dis | cut -c1-16); C=$(grep -c . $C/wait-C.dis) lines sha $(sha256sum < $C/wait-C.dis | cut -c1-16)" INFO; }
# cmp_up_phase: U against P, in its own directories. The differing set must be exactly pthread_cond_wait.o (BCMP-P-EXACT).
cmp_up_phase() { local C=$R/cmp/up a m n names
  V_CMPUP=NOT-RUN
  ok_to_start || { V_CMPUP="NOT-RUN/$REFUSED"; vr X-up-start refused "$V_CMPUP"; return; }
  mkdir -p $C/U $C/P
  for a in U P; do (cd $C/$a && nice -n 10 taskset -c $CORES ar x $R/$a/sysroot/usr/lib/libc.a) || { V_CMPUP=BCMP-INCONCLUSIVE/AR; vr X-up-ar $a $V_CMPUP; return; }
    ar t $R/$a/sysroot/usr/lib/libc.a | sort > $C/members-$a.txt; done
  vr X-up-members "U=$(grep -c . $C/members-U.txt) P=$(grep -c . $C/members-P.txt) dup-U=$(uniq -d $C/members-U.txt | grep -c .) dup-P=$(uniq -d $C/members-P.txt | grep -c .)" INFO
  if [ "$(uniq -d $C/members-U.txt | grep -c .)" != 0 ] || [ "$(uniq -d $C/members-P.txt | grep -c .)" != 0 ] || ! cmp -s $C/members-U.txt $C/members-P.txt; then V_CMPUP=BCMP-INCONCLUSIVE/MEMBERSET; vr X-up-set "$(comm -3 $C/members-U.txt $C/members-P.txt | head -5 | tr '\n' ' ')" $V_CMPUP; return; fi
  for a in U P; do (cd $C/$a && find . -name '*.o' -exec nice -n 10 taskset -c $CORES strip --strip-debug {} +) || { V_CMPUP=BCMP-INCONCLUSIVE/STRIP; vr X-up-strip $a $V_CMPUP; return; }; done
  : > $C/differ.txt; while read -r m; do cmp -s "$C/U/$m" "$C/P/$m" || echo "$m" >> $C/differ.txt; done < $C/members-U.txt
  n=$(grep -c . $C/differ.txt); names=$(tr '\n' ' ' < $C/differ.txt | sed 's/ $//')
  vr X-up-differ "n=$n [$names]" INFO
  if [ "$n" = 0 ]; then V_CMPUP=BCMP-P-NONE
  elif [ "$names" = pthread_cond_wait.o ]; then V_CMPUP=BCMP-P-EXACT
  elif grep -qx pthread_cond_wait.o $C/differ.txt; then V_CMPUP=BCMP-P-EXTRA
  else V_CMPUP=BCMP-P-WRONG; fi
  vr X-up "$V_CMPUP" $V_CMPUP
  for a in U P; do objdump -d --no-show-raw-insn $C/$a/pthread_cond_wait.o > $C/dis-$a.txt 2>&1; done
  vr X-up-disasm "syscall-insns U=$(grep -c 'syscall' $C/dis-U.txt) P=$(grep -c 'syscall' $C/dis-P.txt); imm-0xe6 U=$(grep -cE '\$0xe6,%e' $C/dis-U.txt) P=$(grep -cE '\$0xe6,%e' $C/dis-P.txt)" INFO
  diff -rq $R/U/sysroot/usr/include $R/P/sysroot/usr/include | sed "s|$R/||g" > $C/hdr-diff.txt; vr X-up-hdr "$(grep -c . $C/hdr-diff.txt) lines" INFO; }

# ---- O: the overall code (mechanical; first matching rule wins; the side-effect suffixes are added by the caller)
overall() {
  local v a x
  case $V_TOOLS in TOOLS-BUILT) ;; *) echo INCONCLUSIVE/TOOLS; return;; esac
  for v in "$V_BU" "$V_BC" "$V_BP"; do
    case $v in BUILT) ;; NOT-RUN*|*BUDGET*|*TIMEOUT*) echo INCONCLUSIVE/BUILD; return;; *) echo FAIL-BUILD; return;; esac; done
  case "$V_LU $V_LC $V_LP" in *NOT-RUN*) echo INCONCLUSIVE/LINK; return;; *LINK-FAIL*) echo FAIL-ISOLATION; return;; esac   # LINK before ISO and PLAIN: those are NOT-RUN when the link failed
  if [ "$V_IU/$V_IC/$V_IP" != ISOLATED/ISOLATED/ISOLATED ]; then echo FAIL-ISOLATION; return; fi
  for a in U C P; do x=${VX[CLINK-$a]:-NOT-RUN}
    case $x in CLINK-OK) ;; NOT-RUN*) echo INCONCLUSIVE/CLINK; return;; *) echo FAIL-CLINK; return;; esac; done
  for a in U C P; do x=${VX[CISO-$a]:-NOT-RUN}
    case $x in CISO-OK) ;; NOT-RUN*) echo INCONCLUSIVE/CLINK; return;; *) echo FAIL-CISO; return;; esac; done
  case "$V_PU $V_PC $V_PP" in *NOT-RUN*) echo INCONCLUSIVE/PLAIN; return;; "PLAIN-PASS PLAIN-PASS PLAIN-PASS") ;; *) echo FAIL-PLAIN; return;; esac
  for a in U C P; do x=${VX[CPLAIN-$a]:-NOT-RUN}
    case $x in CPLAIN-PASS|CPLAIN-PASS-VACUOUS) ;; NOT-RUN*) echo INCONCLUSIVE/PLAIN; return;; *) echo FAIL-PLAIN; return;; esac; done
  case $V_CMP in BCMP-EXACT|BCMP-SUBSET*|BCMP-HDR) ;; BCMP-INCONCLUSIVE*|NOT-RUN*) echo INCONCLUSIVE/CMP; return;; *) echo FAIL-ARMS-NOT-AS-EXPECTED; return;; esac
  case $V_CMPUP in BCMP-P-EXACT) ;; BCMP-INCONCLUSIVE*|NOT-RUN*) echo INCONCLUSIVE/CMP; return;; *) echo FAIL-ARMS-NOT-AS-EXPECTED; return;; esac
  if [ "${VX[TFF]:-}" = TFF-STOP ]; then echo STOP-FASTFORWARD; return; fi
  if [ "$V_HU/$V_HC/$V_HP" = HERMIT-PASS/HERMIT-PASS/HERMIT-PASS ] && [ "${VX[TFF]:-}" = TFF-OK ] && [ "${VX[PFF]:-}" = PFF-OK ]; then echo PASS; else echo PARTIAL-HERMIT; fi; }
# side_effects: the suffixes of the overall code, from the before/after files (G0 rev 8 section 9) and the end checks and condlock cells of X1b
side_effects() { local s=
  cmp -s $R/ldcache-before.txt $R/ldcache-post.txt || s="$s+LDCACHE-CHANGED"
  [ -s $R/leftover.txt ] && s="$s+LEFTOVER"
  cmp -s $R/tmp-before.txt $R/tmp-post.txt || s="$s+TMP-LISTING-DIFFERS"
  cmp -s $R/shm-before.txt $R/shm-post.txt || s="$s+SHM-LISTING-DIFFERS"
  cmp -s $R/crash-before.txt $R/crash-post.txt || s="$s+VAR-CRASH-DIFFERS"
  echo "$s$INCH$(cl_suffix)"; }
# overall_line CODE SUFFIXES : the OVERALL row of cases.tsv
overall_line() {
  printf 'OVERALL\t%s%s\ttools=%s build=%s/%s/%s link=%s/%s/%s iso=%s/%s/%s plain=%s/%s/%s clink=%s/%s/%s ciso=%s/%s/%s cplain=%s/%s/%s cmp=%s cmpup=%s hermit=%s/%s/%s tff=%s pff=%s\n' "$1" "$2" "$V_TOOLS" \
    "$V_BU" "$V_BC" "$V_BP" "$V_LU" "$V_LC" "$V_LP" "$V_IU" "$V_IC" "$V_IP" "$V_PU" "$V_PC" "$V_PP" \
    "${VX[CLINK-U]:-NOT-RUN}" "${VX[CLINK-C]:-NOT-RUN}" "${VX[CLINK-P]:-NOT-RUN}" "${VX[CISO-U]:-NOT-RUN}" "${VX[CISO-C]:-NOT-RUN}" "${VX[CISO-P]:-NOT-RUN}" \
    "${VX[CPLAIN-U]:-NOT-RUN}" "${VX[CPLAIN-C]:-NOT-RUN}" "${VX[CPLAIN-P]:-NOT-RUN}" "$V_CMP" "$V_CMPUP" "$V_HU" "$V_HC" "$V_HP" "${VX[TFF]:-NOT-RUN}" "${VX[PFF]:-NOT-RUN}"; }

# ---- test seam: the Mac stub tests source this file with X1B_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${X1B_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "X1B_TEST is set in a real run: refused" >&2; exit 5; fi
  return 0
fi
SID=$(ps -o sid= -p $$ | tr -d ' '); echo "$SID $T0" > $R/sid.txt; echo "gdb=not-used" >> $R/sid.txt
# the driver's whole ancestor chain (this shell up to pid 1), once, as " pid pid ... " (guard.sh of GON/GOG): the launch shell, nohup/timeout and anything above may have the work dir in their arguments
ANC=" "; p=$$
while [ -n "$p" ] && [ "$p" -gt 1 ]; do ANC="$ANC$p "; p=$(ps -o ppid= -p "$p" | tr -d ' '); done
unset p
trap on_signal TERM INT HUP
mkdir -p $R/tmp $H; : > $R/cases.tsv; snapshot pre
cp $R/tmp-pre.txt $R/tmp-before.txt; cp $R/crash-pre.txt $R/crash-before.txt; cp $R/ldcache-pre.txt $R/ldcache-before.txt; cp $R/shm-pre.txt $R/shm-before.txt
V_TOOLS=NOT-RUN; V_BU=NOT-RUN; V_BC=NOT-RUN; V_BP=NOT-RUN; V_LU=NOT-RUN; V_LC=NOT-RUN; V_LP=NOT-RUN; V_IU=NOT-RUN; V_IC=NOT-RUN; V_IP=NOT-RUN
V_PU=NOT-RUN; V_PC=NOT-RUN; V_PP=NOT-RUN; V_HU=NOT-RUN; V_HC=NOT-RUN; V_HP=NOT-RUN; V_CMP=NOT-RUN; V_CMPUP=NOT-RUN

# ---- main
tools_phase; V_TOOLS=$LASTV   # a tools failure is never a glibc failure
[ "$V_TOOLS" = TOOLS-BUILT ] || { vr MAIN stop "$V_TOOLS"; finish_up; printf 'OVERALL\tINCONCLUSIVE/TOOLS\ttools=%s\n' "$V_TOOLS" >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 5; }
src_phase && khdr_phase && ck_extract || { SV=$LASTV; vr MAIN stop "$SV"; finish_up; printf 'OVERALL\t%s\n' "$([[ $SV == NOT-RUN* ]] && echo INCONCLUSIVE/SOURCES || echo FAIL-SOURCES)" >> $R/cases.tsv; echo DONE-OVERALL >> $R/steps.tsv; exit 5; }
build_arm U; V_BU=$LASTV
if [ "$V_BU" = BUILT ]; then
  if [ $(( $(date +%s) + BW * 12 / 10 )) -le $BDL ]; then build_arm C; V_BC=$LASTV
  else vr B-C "first build took ${BW}s; a second build of the same size cannot finish by the build deadline (T0+45 min)" NOT-RUN/BUDGET; V_BC=$LASTV; fi
fi
if [ "$V_BC" = BUILT ]; then
  if [ $(( $(date +%s) + BW * 12 / 10 )) -le $BDL ]; then build_arm P; V_BP=$LASTV
  else vr B-P "the previous build took ${BW}s; one more of the same size cannot finish by the build deadline (T0+45 min)" NOT-RUN/BUDGET; V_BP=$LASTV; fi
fi
STEPCLASS=A
if [ "$V_BU" = BUILT ] && [ "$V_BC" = BUILT ] && [ "$V_BP" = BUILT ]; then
  libc_ref
  link_arm U; V_LU=$LASTV; link_arm C; V_LC=$LASTV; link_arm P; V_LP=$LASTV; : > $R/binaries.sha
  [[ $V_LU == LINK-OK-L* ]] && { iso_arm U; V_IU=$LASTV; }
  [[ $V_LC == LINK-OK-L* ]] && { iso_arm C; V_IC=$LASTV; }
  [[ $V_LP == LINK-OK-L* ]] && { iso_arm P; V_IP=$LASTV; }
  mkdir -p $R/SYS
  if step SYS-build 300 $R/SYS gcc -O2 -g -pthread -static -o $R/SYS/condsmoke-SYS $BIN/condsmoke.c; then
    rc=$(runone plain-SYS 10 $R/SYS/condsmoke-SYS); vr P-SYS "rc=$rc $(cat $L/plain-SYS.out)" "$([ "$rc" = 0 ] && [ "$(cat $L/plain-SYS.out)" = "$SYS_LINE" ] && echo CONTROL-OK || echo CONTROL-FAIL)"
  else vr SYS-build fail CONTROL-NOT-BUILT; fi
  [ "$V_IU" = ISOLATED ] && { plain_arm U $R/U/smoke/condsmoke-U "$(WANT_LINE)"; V_PU=$LASTV; }
  [ "$V_IC" = ISOLATED ] && { plain_arm C $R/C/smoke/condsmoke-C "$(WANT_LINE)"; V_PC=$LASTV; }
  [ "$V_IP" = ISOLATED ] && { plain_arm P $R/P/smoke/condsmoke-P "$(WANT_LINE)"; V_PP=$LASTV; }
  cmp_phase
  cmp_up_phase
  for a in U C P; do ck_arm $a && ciso_arm $a; done
  for a in U C P; do [ "${VX[CISO-$a]:-}" = CISO-OK ] && clplain_arm $a; done
  if [ "$V_PU" = PLAIN-PASS ] && [ "$V_PC" = PLAIN-PASS ] && [ "$V_PP" = PLAIN-PASS ]; then
    hermit_phase; if [ "$V_HU" = NOT-RUN ]; then case $LASTV in HERMIT-NOT-RUN*) V_HU=$LASTV; V_HC=$LASTV; V_HP=$LASTV;; esac; fi
    tprobe_phase; pff_eval; clh_phase
  fi
fi
finish_up
# ---- side effects and overall (mechanical; the result is copied, not re-judged)
overall_line "$(overall)" "$(side_effects)" >> $R/cases.tsv
echo DONE-OVERALL >> $R/steps.tsv
