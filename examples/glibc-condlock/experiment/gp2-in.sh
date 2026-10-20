#!/bin/bash
# G0p2 in-container script. Runs as root inside ubuntu:24.04 (linux/amd64). Writes only /out (the bind mount) and /root/m0 (the container's own layer, lost at exit).
# usage: bash /bin-gp/gp2-in.sh <DEADLINE_EPOCH>
# inputs (read-only): /in/glibc-U.tar /in/glibc-P2.tar /in/positive-control-p2.patch /in/condlock.tar
# Every result is a line "<key><TAB><value>" in /out/status.tsv; a host-side driver (not published: it only wrapped Docker and read these keys) turned them into the verdict. This script never decides PASS.
# R is /root/m0 on purpose (the M0 path), so that the U libc.a hash can be compared with the M0 rerun's (information only).
# O, R and IN can be moved through G0P2_OUT, G0P2_R and G0P2_IN: a seam for the stub tests only; the container run sets none of them.
set -u
DEADLINE=${1:-0}
O=${G0P2_OUT:-/out}; R=${G0P2_R:-/root/m0}; IN=${G0P2_IN:-/in}; L=$O/logs; CK=$R/ck; CLK=$CK/examples/glibc-condlock
GCC_VER='13.3.0-6ubuntu2~24.04.1'
SPECS_WANT='%{!O0:%{O*:%{!D_FORTIFY_SOURCE=*:%{!U_FORTIFY_SOURCE:-D_FORTIFY_SOURCE=3}}}}'
PIN_PKGS="gcc-13=$GCC_VER g++-13=$GCC_VER cpp-13=$GCC_VER gcc-13-base=$GCC_VER libgcc-13-dev=$GCC_VER libstdc++-13-dev=$GCC_VER"
OTHER_PKGS="gcc g++ cpp make bison m4 gawk python3 perl sed binutils libc6-dev linux-libc-dev file patch"
SRC_COUNT=${G0P2_SRC_COUNT:-19756}
U_FILE_SHA=55ce6de06fc59642cae8a82c63c683fb39856fdcf5a6cbc5b3940ba8cabc7e25
P2_FILE_SHA=2d28a8cd2f648b1beea405d3ae23c33c1a767c9a894985e9d3d37565358afd05
U_BLOB=66786c7b9022b26c7bc02defcb576e967b1ad335
P2_BLOB=e58d3b839cb906ca73f0b5408634e45db8ab6456
MARKER=P2-POSITIVE-CONTROL-HUNK
G1LINE='uint64_t g1_start = __condvar_load_g1_start_relaxed (cond);'
FORLINE='for (unsigned int p2_spin = 0; p2_spin < 40000; p2_spin++)'
ASMLINE='__asm__ __volatile__ ("pause" ::: "memory");'
ARMDEF=CONDLOCK_ARM_U      # P2 has the U layout (same headers as U)
SMOKE_CAP=60                # T-probe runs only: the native condlock runs of G0p2 rev 1a were dropped in rev 2 (no P2 outcome data before the Hermit probe)
MIN_PASS_BRANCHES=10000     # graded: conditional branches per pass in every inlined copy (design 4.1: one chaos cap)
export DEBIAN_FRONTEND=noninteractive LC_ALL=C

st() { printf '%s\t%s\n' "$1" "$2" >> $O/status.tsv; }
fin() { st DONE "$1"; exit 0; }
step() { # id timeout cwd cmd... : one logged step, clean env
  local id=$1 to=$2 cwd=$3 t0 rc; shift 3
  t0=$(date +%s)
  ( ulimit -c 0; cd "$cwd" && env -i PATH=/usr/bin:/bin HOME=$R LANG=C LC_ALL=C TMPDIR=$R/tmp timeout -k 30 "$to" "$@" ) > $L/$id.log 2>&1
  rc=$?
  printf '%s\t%s\t%s\t%s\n' "$id" "$rc" "$(( $(date +%s) - t0 ))" "$*" >> $O/steps.tsv
  return $rc; }
runone() { # id timeout cmd... : one smoke run, prints the exit status; stdout and stderr go to /out/smoke
  local id=$1 tmo=$2; shift 2
  ( ulimit -c 0; cd $CK && env -i PATH=/usr/bin:/bin HOME=$R LANG=C LC_ALL=C timeout -k 5 "$tmo" "$@" ) > $O/smoke/$id.out 2> $O/smoke/$id.err < /dev/null
  echo $?; }
blobsha() { { printf 'blob %s\0' "$(wc -c < "$1" | tr -d ' ')"; cat "$1"; } | sha1sum | cut -d' ' -f1; }   # git blob id of a file

# ---- environment, packages and gates (as M0 and G0p)
prep_phase() { local rc p v d1 d2 d3
  mkdir -p $L $R/tmp $R/khdr $O/smoke
  { uname -m; nproc; cat /sys/fs/cgroup/cpu.max /sys/fs/cgroup/memory.max; head -3 /etc/os-release; date -u +%FT%TZ; } > $O/env.txt 2>&1
  st ARCH "$(uname -m)"
  timeout -k 10 600 apt-get update > $L/apt-update.log 2>&1; rc=$?; st APT-UPDATE $rc
  [ $rc = 0 ] || fin gate-apt-update
  # shellcheck disable=SC2086  # the package lists are deliberate word lists
  timeout -k 10 900 apt-get install -y --no-install-recommends $PIN_PKGS > $L/apt-pin.log 2>&1; rc=$?; st APT-PIN $rc
  if [ $rc != 0 ]; then
    if grep -q -E "was not found|has no installation candidate|Unable to locate|Version '.*' for" $L/apt-pin.log; then st GCC-VERSION not-installable; else st APT-PIN-ERR other; fi
    fin gate-apt-pin; fi
  # shellcheck disable=SC2086
  timeout -k 10 900 apt-get install -y --no-install-recommends $OTHER_PKGS > $L/apt-other.log 2>&1; rc=$?; st APT-OTHER $rc
  [ $rc = 0 ] || fin gate-apt-other
  for p in gcc-13 g++-13 cpp-13 gcc-13-base libgcc-13-dev libstdc++-13-dev; do
    v=$(dpkg-query -W -f='${Version}' $p 2>&1); st "PKG-$p" "$v"
    [ "$v" = "$GCC_VER" ] || { st GCC-VERSION "$p=$v"; fin gate-gcc-version; }; done
  st GCC-VERSION ok
  dpkg-query -W gcc-13 g++-13 cpp-13 binutils libc6 libc6-dev linux-libc-dev gawk bison m4 make python3 perl sed patch > $O/dpkg.txt 2>&1
  { gcc --version | head -1; ld --version | head -1; bison --version | head -1; m4 --version | head -1; make --version | head -1; gawk --version | head -1; patch --version | head -1
    command -v makeinfo || echo "makeinfo absent"; } > $O/tools.txt 2>&1
  st BINUTILS "$(dpkg-query -W -f='${Version}' binutils 2>&1)"
  gcc -dumpspecs | grep -n FORTIFY > $O/specs-fortify.txt 2>&1
  grep -q -F -- "$SPECS_WANT" $O/specs-fortify.txt || { st SPECS missing; fin gate-specs; }
  d1=$(echo | gcc -O2 -dM -E - | grep -c '_FORTIFY_SOURCE 3')
  d2=$(echo | gcc -O2 -Wp,-U_FORTIFY_SOURCE -dM -E - | grep -c '_FORTIFY_SOURCE 3')
  d3=$(echo | gcc -O2 -U_FORTIFY_SOURCE -dM -E - | grep -c '_FORTIFY_SOURCE')
  st SPECS-PROBE "O2=$d1 Wp-U=$d2 U=$d3"
  [ "$d1/$d2/$d3" = 1/1/0 ] || { st SPECS probe-mismatch; fin gate-specs; }
  st SPECS ok
  [ "$(bison --version | head -n 1)" = "bison (GNU Bison) 3.8.2" ] && [ "$(m4 --version | head -n 1)" = "m4 (GNU M4) 1.4.19" ] && [ "$(gcc -dumpfullversion)" = 13.3.0 ] || { st TOOLS mismatch; fin gate-tools; }
  st TOOLS ok
  if ! { cp -a /usr/include/linux $R/khdr/linux && cp -a /usr/include/asm-generic $R/khdr/asm-generic && cp -a /usr/include/x86_64-linux-gnu/asm $R/khdr/asm; }; then st KHDR fail; fin gate-khdr; fi
  return 0; }

# ---- inputs: the two trees, the patch, the condlock tree; the pinned facts about P2 (nothing is built yet)
inputs_phase() { local a nu np pu pp bu bp bad='' rc ad de
  for a in P2 U; do mkdir -p $R/$a/build; tar -xf $IN/glibc-$a.tar -C $R/$a || { st "EXTRACT-$a" fail; return 1; }; done
  mkdir -p $CK; tar -xf $IN/condlock.tar -C $CK || { st EXTRACT-CLK fail; return 1; }
  nu=$(find $R/U/src \( -type f -o -type l \) | wc -l | tr -d ' '); np=$(find $R/P2/src \( -type f -o -type l \) | wc -l | tr -d ' ')
  st SRC-COUNT "U=$nu P2=$np want=$SRC_COUNT"; [ "$nu" = "$SRC_COUNT" ] && [ "$np" = "$SRC_COUNT" ] || bad="$bad count"
  diff -rq $R/U/src $R/P2/src | sed "s|$R/||g" > $O/srcdiff-UP2.txt
  [ "$(cat $O/srcdiff-UP2.txt)" = "Files U/src/nptl/pthread_cond_wait.c and P2/src/nptl/pthread_cond_wait.c differ" ] || bad="$bad srcdiff"
  pu=$(sha256sum $R/U/src/nptl/pthread_cond_wait.c | cut -d' ' -f1); pp=$(sha256sum $R/P2/src/nptl/pthread_cond_wait.c | cut -d' ' -f1)
  st FILE-SHA "U=$pu P2=$pp"; [ "$pu" = $U_FILE_SHA ] && [ "$pp" = $P2_FILE_SHA ] || bad="$bad filesha"
  bu=$(blobsha $R/U/src/nptl/pthread_cond_wait.c); bp=$(blobsha $R/P2/src/nptl/pthread_cond_wait.c)
  st BLOB-SHA "U=$bu P2=$bp"; [ "$bu" = $U_BLOB ] && [ "$bp" = $P2_BLOB ] || bad="$bad blob"
  st MARKER-COUNT "P2=$(grep -cF "$MARKER" $R/P2/src/nptl/pthread_cond_wait.c) U=$(grep -cF "$MARKER" $R/U/src/nptl/pthread_cond_wait.c)"
  [ "$(grep -cF "$MARKER" $R/P2/src/nptl/pthread_cond_wait.c)" = 1 ] && [ "$(grep -cF "$MARKER" $R/U/src/nptl/pthread_cond_wait.c)" = 0 ] || bad="$bad marker"
  st HUNK-LINES "for P2=$(grep -cF "$FORLINE" $R/P2/src/nptl/pthread_cond_wait.c) U=$(grep -cF "$FORLINE" $R/U/src/nptl/pthread_cond_wait.c); asm P2=$(grep -cF "$ASMLINE" $R/P2/src/nptl/pthread_cond_wait.c) U=$(grep -cF "$ASMLINE" $R/U/src/nptl/pthread_cond_wait.c)"
  [ "$(grep -cF "$FORLINE" $R/P2/src/nptl/pthread_cond_wait.c)" = 1 ] && [ "$(grep -cF "$FORLINE" $R/U/src/nptl/pthread_cond_wait.c)" = 0 ] && [ "$(grep -cF "$ASMLINE" $R/P2/src/nptl/pthread_cond_wait.c)" = 1 ] && [ "$(grep -cF "$ASMLINE" $R/U/src/nptl/pthread_cond_wait.c)" = 0 ] || bad="$bad hunklines"
  st G1LINE-COUNT "P2=$(grep -cF "$G1LINE" $R/P2/src/nptl/pthread_cond_wait.c) U=$(grep -cF "$G1LINE" $R/U/src/nptl/pthread_cond_wait.c)"
  [ "$(grep -cF "$G1LINE" $R/P2/src/nptl/pthread_cond_wait.c)" = 1 ] && [ "$(grep -cF "$G1LINE" $R/U/src/nptl/pthread_cond_wait.c)" = 1 ] || bad="$bad g1line"
  [ "$(grep -c '^diff --git ' $IN/positive-control-p2.patch)" = 1 ] && [ "$(grep -c '^+++ ' $IN/positive-control-p2.patch)" = 1 ] && grep -qx '+++ b/nptl/pthread_cond_wait.c' $IN/positive-control-p2.patch || bad="$bad patchfiles"
  ad=$(grep '^+' $IN/positive-control-p2.patch | grep -vc '^+++ '); de=$(grep '^-' $IN/positive-control-p2.patch | grep -vc '^--- ')
  st PATCH-LINES "added=$ad deleted=$de"; [ "$ad" = 10 ] && [ "$de" = 0 ] || bad="$bad patchlines"
  mkdir -p $R/pchk/nptl; cp $R/U/src/nptl/pthread_cond_wait.c $R/pchk/nptl/pthread_cond_wait.c
  step patch-apply 60 $R/pchk patch -p1 -i $IN/positive-control-p2.patch; rc=$?
  st PATCH-APPLY "rc=$rc fuzz-or-offset-lines=$(grep -c -E 'fuzz|offset' $L/patch-apply.log)"
  [ $rc = 0 ] && [ "$(grep -c -E 'fuzz|offset' $L/patch-apply.log)" = 0 ] || bad="$bad patchapply"
  [ "$(sha256sum $R/pchk/nptl/pthread_cond_wait.c | cut -d' ' -f1)" = $P2_FILE_SHA ] || bad="$bad patchresult"
  if [ -n "$bad" ]; then st INPUTS "bad:$bad"; return 1; fi
  st INPUTS ok; return 0; }

# ---- one arm: the G0 rev 8 recipe (same configure line as g0.sh and M0), make -j8
build_arm() { local a=$1 rc to f t0 S miss=''
  S=$R/$a/sysroot; t0=$(date +%s)
  step cfg-$a 1500 $R/$a/build ../src/configure CC=gcc "CFLAGS=-O2 -g -U_FORTIFY_SOURCE -ffile-prefix-map=$R/$a=/g0" MAKEINFO=: --prefix=/usr --libdir=/usr/lib --with-headers=$R/khdr --disable-werror --disable-profile --disable-nscd --without-selinux; rc=$?
  st "CFG-$a" $rc; [ $rc = 0 ] || return 1
  to=$((DEADLINE - $(date +%s))); [ "$to" -gt 300 ] || { st "MAKE-$a" NOT-RUN-BUDGET; return 1; }
  step mk-$a "$to" $R/$a/build make -j8; rc=$?
  st "MAKE-$a" $rc
  grep -m1 '^gcc syslog.c ' $L/mk-$a.log > $O/compile-syslog-$a.txt
  if [ -s $O/compile-syslog-$a.txt ] && grep -q -F -e ' -U_FORTIFY_SOURCE ' $O/compile-syslog-$a.txt; then st "CLINE-$a" ok; else st "CLINE-$a" missing; fi
  st "WARN-$a" "$(grep -c 'warning:' $L/mk-$a.log)"
  [ $rc = 0 ] || return 1
  to=$((DEADLINE - $(date +%s))); [ "$to" -gt 120 ] || { st "INSTALL-$a" NOT-RUN-BUDGET; return 1; }
  step in-$a "$to" $R/$a/build make install install_root=$S; rc=$?
  st "INSTALL-$a" $rc; [ $rc = 0 ] || return 1
  step khdr-in-$a 300 $R cp -a $R/khdr/. $S/usr/include/ || { st "LAYOUT-$a" khdr-copy-fail; return 1; }
  for f in usr/lib/libc.a usr/lib/crt1.o usr/lib/crti.o usr/lib/crtn.o usr/include/gnu/libc-version.h usr/include/pthread.h usr/include/bits/thread-shared-types.h; do [ -f $S/$f ] || miss="$miss $f"; done
  if [ -n "$miss" ]; then st "LAYOUT-$a" "missing:$miss"; return 1; fi
  st "LAYOUT-$a" OK
  st "LIBC-SHA-$a" "$(sha256sum $S/usr/lib/libc.a | cut -d' ' -f1)"
  st "LIBC-MEMBERS-$a" "$(ar t $S/usr/lib/libc.a | wc -l | tr -d ' ')"
  st "WALL-$a" "$(( $(date +%s) - t0 ))"
  return 0; }

# ---- U against P2: libc.a members with debug info stripped (G0 rev 8 section 6.6); the differing set must be exactly pthread_cond_wait.o
cmp_up2() { local C=$R/cmp a m n names
  mkdir -p $C/U $C/P2
  for a in U P2; do
    ( cd $C/$a && ar x $R/$a/sysroot/usr/lib/libc.a ) > $L/ar-$a.log 2>&1 || { st BCMP INCONCLUSIVE/AR; return 1; }
    ar t $R/$a/sysroot/usr/lib/libc.a | sort > $C/members-$a.txt; done
  st BCMP-MEMBERS "U=$(grep -c . $C/members-U.txt) P2=$(grep -c . $C/members-P2.txt) dup-U=$(uniq -d $C/members-U.txt | grep -c .) dup-P2=$(uniq -d $C/members-P2.txt | grep -c .)"
  if [ "$(uniq -d $C/members-U.txt | grep -c .)" != 0 ] || [ "$(uniq -d $C/members-P2.txt | grep -c .)" != 0 ] || ! cmp -s $C/members-U.txt $C/members-P2.txt; then st BCMP INCONCLUSIVE/MEMBERSET; return 1; fi
  for a in U P2; do ( cd $C/$a && find . -name '*.o' -exec strip --strip-debug {} + ) > $L/strip-$a.log 2>&1 || { st BCMP INCONCLUSIVE/STRIP; return 1; }; done
  : > $C/differ.txt
  while read -r m; do cmp -s "$C/U/$m" "$C/P2/$m" || echo "$m" >> $C/differ.txt; done < $C/members-U.txt
  n=$(grep -c . $C/differ.txt); names=$(tr '\n' ' ' < $C/differ.txt | sed 's/ $//')
  st BCMP-DIFFERS "n=$n [$names]"
  if [ "$n" = 0 ]; then st BCMP BCMP-P2-NONE
  elif [ "$names" = pthread_cond_wait.o ]; then st BCMP BCMP-P2-EXACT
  elif grep -qx pthread_cond_wait.o $C/differ.txt; then st BCMP BCMP-P2-EXTRA
  else st BCMP BCMP-P2-WRONG; fi
  diff -rq $R/U/sysroot $R/P2/sysroot | sed "s|$R/||g" > $O/tree-diff-UP2.txt; st TREE-DIFF-UP2 "$(grep -c . $O/tree-diff-UP2.txt) lines"
  if cmp -s $R/U/sysroot/usr/include/pthread.h $R/P2/sysroot/usr/include/pthread.h && cmp -s $R/U/sysroot/usr/include/bits/thread-shared-types.h $R/P2/sysroot/usr/include/bits/thread-shared-types.h; then st HDR-UP2-EQ yes; else st HDR-UP2-EQ no; fi
  return 0; }

# ---- disassembly of the stripped pthread_cond_wait.o of U and P2 (spec section 1, Q3)
mn() { # file ere : number of instructions whose mnemonic (with an optional lock prefix as written in the ere) matches
  grep -cE "^[[:space:]]+[0-9a-f]+:[[:space:]]+($2)([[:space:]]|\$)" "$1"; }
INV_COUNTS=''; INV_DIFF=''
invk() { # key ere : compare the U and P2 counts of one instruction class
  local cu cp2
  cu=$(mn $R/cmp/dis-U.txt "$2"); cp2=$(mn $R/cmp/dis-P2.txt "$2")
  INV_COUNTS="$INV_COUNTS $1 U=$cu P2=$cp2;"
  [ "$cu" = "$cp2" ] || INV_DIFF="$INV_DIFF $1"; }
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
order_status() { # S-lines : ok / hoisted / pattern / no-loops
  local f=$1 nl h nc na
  nl=$(grep -c . "$f"); h=$(grep -c ' hoist=1 ' "$f"); nc=$(grep -c ' cas=0 ' "$f"); na=$(grep -c ' after=0 ' "$f")
  if [ "$nl" = 0 ]; then echo "no-loops"
  elif [ "$h" != 0 ]; then echo "hoisted loops=$nl hoist=$h"
  elif [ "$nc" != 0 ] || [ "$na" != 0 ]; then echo "pattern loops=$nl no-cas=$nc no-load-after=$na"
  else echo "ok loops=$nl"; fi; }
loop_cbr_status() { # S-lines : every loop must have 1 or more conditional branches per iteration (the exact counts are recorded); 0 or an undelimited body (-1) is not ok
  local f=$1 nl bad list
  nl=$(grep -c . "$f"); bad=$(grep -c -E ' cbr=(0|-1) ' "$f")
  list=$(sed -n 's/.* cbr=\([-0-9]*\) .*/\1/p' "$f" | tr '\n' ',' | sed 's/,$//')
  if [ "$nl" = 0 ]; then echo "no-loops"; elif [ "$bad" != 0 ]; then echo "zero-or-unparsable loops=$nl cbr=$list"; else echo "ok loops=$nl cbr=$list"; fi; }
loop_perpass_status() { # S-lines : graded: branches per pass >= MIN_PASS_BRANCHES in every inlined copy; any unrecognised counter form (perpass=-1) is reported as such
  local f=$1 nl un low list
  nl=$(grep -c . "$f"); un=$(grep -c ' perpass=-1$' "$f")
  low=$(awk -v m="$MIN_PASS_BRANCHES" '{ for (i = 1; i <= NF; i++) if ($i ~ /^perpass=/) { v = substr($i, 9) + 0; if (v >= 0 && v < m) n++ } } END { print n + 0 }' "$f")
  list=$(sed -n 's/.* perpass=\([-0-9]*\)$/\1/p' "$f" | tr '\n' ',' | sed 's/,$//')
  if [ "$nl" = 0 ]; then echo "no-loops"
  elif [ "$un" != 0 ]; then echo "unrecognised loops=$nl perpass=$list"
  elif [ "$low" != 0 ]; then echo "below loops=$nl min=$MIN_PASS_BRANCHES perpass=$list"
  else echo "ok loops=$nl min=$MIN_PASS_BRANCHES perpass=$list"; fi; }
loop_const_status() { # S-lines : the loop constant must be in or just before every loop body
  local f=$1 nl bad list
  nl=$(grep -c . "$f"); bad=$(grep -c ' const=0' "$f")
  list=$(sed -n 's/.* const=\([01]\).*/\1/p' "$f" | tr '\n' ',' | sed 's/,$//')
  if [ "$nl" = 0 ]; then echo "no-loops"; elif [ "$bad" != 0 ]; then echo "missing loops=$nl const=$list"; else echo "ok loops=$nl const=$list"; fi; }
disasm_phase() { local C=$R/cmp a pu pp
  INV_COUNTS=''; INV_DIFF=''
  for a in U P2; do
    objdump -d --no-show-raw-insn $C/$a/pthread_cond_wait.o > $C/dis-$a.txt 2> $L/objdump-$a.log || { st DISASM-ORDER objdump-failed; return 1; }
    cp $C/dis-$a.txt $O/dis-$a.txt; done
  pu=$(mn $C/dis-U.txt pause); pp=$(mn $C/dis-P2.txt pause)
  st PAUSE-U "$pu"; st PAUSE-P2 "$pp"
  invk syscall syscall; invk lock lock; invk fence 'mfence|lfence|sfence'; invk cmpxchg '(lock[[:space:]]+)?cmpxchg'
  st DISASM-COUNTS "$INV_COUNTS"
  if [ -z "$INV_DIFF" ]; then st DISASM-INV ok; else st DISASM-INV "diff:$INV_DIFF"; fi
  dis_order $C/dis-P2.txt > $O/dis-loops-P2.raw
  grep '^S ' $O/dis-loops-P2.raw | cut -c3- > $O/dis-loops-P2.txt
  grep '^D ' $O/dis-loops-P2.raw | cut -c3- > $O/loop-bodies-P2.txt
  st PAUSE-LOOPS "$(grep -c . $O/dis-loops-P2.txt)"
  st DISASM-ORDER "$(order_status $O/dis-loops-P2.txt)"
  st LOOP-CBR "$(loop_cbr_status $O/dis-loops-P2.txt)"
  st LOOP-PERPASS "$(loop_perpass_status $O/dis-loops-P2.txt)"
  st LOOP-CONST "$(loop_const_status $O/dis-loops-P2.txt)"
  st LOOP-CONST-OBJ "0x9c40-or-0x9c3f lines P2=$(grep -cE '0x9c40|0x9c3f' $C/dis-P2.txt) U=$(grep -cE '0x9c40|0x9c3f' $C/dis-U.txt)"
  return 0; }

# ---- compile checks that could not be done on the Mac, against the real glibc 2.40 headers of the container's sysroots
inc_viol() { # log sysroot : absolute include paths in a gcc -M log that are neither in the sysroot, nor gcc's own, nor the condlock tree
  tr -s ' \\' '\n' < $L/$1.log | grep '^/' | grep -v -e "^$2/" -e "^$(realpath "$2")/" -e '^/usr/lib/gcc/x86_64-linux-gnu/13/' -e "^$CK/" | wc -l | tr -d ' '; }
condraw_check() { # id sysroot-arm define : compile cl_condraw.c; prints nothing, sets status CHK-<id> = rc, INC-<id>, BRANCH-<id>
  local id=$1 def=$3 S=$R/$2/sysroot rc fl
  fl="--sysroot=$S -B$S/usr/lib/ -std=gnu11 -O2 -g -Wall -Wextra -Werror -pthread -Isrc -D$def"
  # shellcheck disable=SC2086  # fl is a deliberate word list
  step "co-$id" 300 $CLK gcc $fl -c -o $CK/co-$id.o src/cl_condraw.c; rc=$?
  st "CHK-$id" $rc
  # shellcheck disable=SC2086
  step "co-$id-M" 120 $CLK gcc $fl -M src/cl_condraw.c; st "INC-$id" "$(inc_viol "co-$id-M" "$S")"
  # shellcheck disable=SC2086
  step "co-$id-E" 120 $CLK gcc $fl -E src/cl_condraw.c
  st "BRANCH-$id" "arm_u=$(grep -c 'raw.g_refs\[0\] = c->__data.__g_refs\[0\]' $L/co-$id-E.log) arm_c=$(grep -c 'raw.arm = CL_ARM_C;' $L/co-$id-E.log) value64=$(grep -c 'raw.wseq = c->__data.__wseq.__value64' $L/co-$id-E.log)"; }
workload_arm() { # arm : the SDK for this arm's headers, tprobe, then condlock, linked static against the arm's sysroot (statuses SDK-, TPROBE-, LINK-, ISO-)
  local a=$1 S=$R/$1/sysroot B=$CK/b-$1 cc rc viol libc_n sdk_n
  cc="gcc --sysroot=$S -B$S/usr/lib/ -static -L$S/usr/lib -Wl,-t,-t"
  mkdir -p $B
  step sdk-$a 900 $CLK make -s sdk "CC=$cc" "BUILD=$B"; rc=$?; st "SDK-$a" $rc
  [ -f $B/sdk/libhairspring.a ] && st "SDKSHA-$a" "$(sha256sum $B/sdk/libhairspring.a | cut -d' ' -f1)"
  step tprobe-$a 300 $CLK make -s tprobe "CC=$cc" "BUILD=$B"; st "TPROBE-$a" $?
  step link-$a 900 $CLK env "CFLAGS=-D$ARMDEF" make -s condlock "CC=$cc" "BUILD=$B"; rc=$?; st "LINK-$a" $rc
  [ $rc = 0 ] || return 1
  grep -v '^attempt to open .* failed' $L/link-$a.log > $L/iso-$a.tr
  { grep -E '(crt1|crti|crtn|Scrt1|rcrt1|gcrt1)\.o|lib(c|m|pthread|dl|rt|util|resolv|anl|mvec|c_nonshared)\.(a|so)|ld-linux' $L/iso-$a.tr | grep -vF "$S/"; grep -E '/(usr/)?lib(64)?/x86_64-linux-gnu/' $L/iso-$a.tr; } > $L/iso-$a.txt
  viol=$(grep -c . $L/iso-$a.txt); libc_n=$(grep -cF -e "$S/usr/lib/libc.a(" -e "($S/usr/lib/libc.a)" $L/iso-$a.tr); sdk_n=$(grep -cF 'libhairspring.a' $L/iso-$a.tr)
  st "ISO-$a" "violations=$viol libc_member_lines=$libc_n sdk_lines=$sdk_n"
  { readelf -lW $B/condlock | grep -c INTERP; readelf -dW $B/condlock 2>&1 | head -n 1; strings -a $B/condlock | grep -c 'Ubuntu GLIBC'; sha256sum $B/condlock; } > $L/elf-$a.txt 2>&1
  return 0; }

# ---- informational plain runs: the three T-probe cases only (rev 2 runs no condlock: no P2 outcome data before the Hermit probe); outputs are recorded, nothing is graded here
smoke_arm() { local a=$1 B=$CK/b-$1 rc k line noexec=0
  if [ $((DEADLINE - $(date +%s))) -le 300 ]; then st "SMOKE-$a" NOT-RUN-BUDGET; return 1; fi
  for k in "i 1" "iii 1" "ii 1000"; do
    # shellcheck disable=SC2086  # k is a deliberate word list (case and argument)
    rc=$(runone "tprobe-$a-${k%% *}" $SMOKE_CAP $B/tprobe $k); line=$(head -n 1 $O/smoke/tprobe-$a-${k%% *}.out)
    printf '%s\ttprobe %s\trc=%s\t%s\n' "$a" "$k" "$rc" "$line" >> $O/smoke.tsv
    { [ "$rc" = 126 ] || [ "$rc" = 127 ]; } && noexec=1; done
  if [ $noexec = 1 ]; then st "SMOKE-$a" noexec; else st "SMOKE-$a" ran; fi
  return 0; }

checks_phase() { local a
  cp $CLK/PINS $O/clk-pins.txt
  condraw_check U-on-U U $ARMDEF
  condraw_check U-on-P2 P2 $ARMDEF
  for a in P2 U; do workload_arm $a; done
  for a in P2 U; do [ "$(awk -F'\t' -v k="LINK-$a" '$1==k{print $2}' $O/status.tsv)" = 0 ] && smoke_arm $a; done
  return 0; }

finish_all() { st DISK-KB "$(du -sk $R | cut -f1)"; fin "$1"; }
main() {
  prep_phase
  inputs_phase || finish_all gate-inputs
  build_arm P2 || finish_all stop-after-p2        # P2 is the subject of the gate: if it fails nothing else is built
  build_arm U || finish_all stop-after-u
  cmp_up2
  disasm_phase
  checks_phase
  finish_all all; }

if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
