#!/usr/bin/env bash
# x2.sh : X2 driver, native rates of the glibc `condlock` workload on the box. uid 1000, NO sudo, NO Hermit, no host change.
# usage: taskset -c 0 setsid nohup timeout -k 30 5700 bash $R/bin/x2.sh $R > $R/run.out 2>&1 < /dev/null &
# Cells, in order: NP (arm P, NP_RUNS serial runs), NA (arms U and C, one run of each per round, the starting arm alternating, rounds until the NA deadline).
# Every run is `env -i ... nice -n 10 taskset -c 1,2,3 timeout -k 5 60 <binary> <arm's arguments>` as uid 1000, one at a time. Nothing here stops on a result:
# only the safety conditions of ok_to_start (guard.sh) and the NA deadline end work. Classes come from the repo's own scripts/classify.sh (X1b's), never from an engine verdict.
# The binaries are the X1b stamp's (reused after sha256 verification, copied into $R/g); this script builds nothing and installs nothing.
set -u
R=$1
T0=$(date +%s)
NEWBY=$((T0 + 90 * 60))                  # no new unit of work after 90 min
ADEAD=$((T0 + 80 * 60))                  # no new NA round after 80 min
BIN=$R/bin; G=$R/g; O=$R/out; RUNS=$R/runs; RAW=$R/raw; CUR=$R/cur/c; CLS=$BIN/classify.sh
CORES=1,2,3; CPATH=/usr/bin:/bin; CAP=60; OK_EVERY=10
NP_RUNS=200
ARGS_UC="-n 8 -r 5000 -tick-ms 50 -q 10"     # arms U and C: exactly X1b's cell (the value was fixed before any box data)
ARGS_P="-n 8 -r 60 -tick-ms 50 -q 10"        # arm P: its own pre-registered setting (X1b's); never compared in rate with U or C
# Pins (spec section 3). The X1b work dir is only read: sha256sum of the files below and cp of the three binaries.
X1B=/path/to/x1b-glibc-20261017T1441Z   # EDIT: the work dir of x1b.sh (stamp 20261017T1441Z in the published run)
PIN_CLS=5fbc06df3de905c7c67a745d1b8c93c62d886c182f98ac85f93c92433637d32a
PIN_X1B_CASES=0fd5a4a8f34c550d4e81d1d890ae1fa3fb40918e8f98bd02059f79a37e082eb1
PIN_X1B_STEPS=ea10be71b3e814b697d13db4068734cf7dfb805e06bb1b0b18b9093e0b485604
# shellcheck source=guard.sh
. "$R/bin/guard.sh"

RE_OUT='^condlock libc=([^ ]+) hdr=([^ ]+) rounds= *([0-9]+) signals= *([0-9]+) blocked1= *([0-9]+) maxb= *([0-9]+) maxstag= *([0-9]+) ticks= *([0-9]+) spanned= *([0-9]+)$'
RE_CLS='^class=([A-Z-]+) subtype=([A-Za-z+-]+) prop_false=([01]) stall_hit=([01]) applied=([01-]) hit_also=([01]) detail='
declare -A KEPT_SAMPLE

sha_of() { sha256sum "$1" 2> /dev/null | cut -d' ' -f1; }
# pin_of BIN|LIBC u|c|p : the condlock binary of the X1b stamp (ck/b-<arm>/condlock, X1b binaries.sha), or the arm's sysroot libc.a (X1b cases.tsv B-libc-<arm>)
pin_of() { case "$1_$2" in
  BIN_u)  echo 9a34111a27d44d28b580b09cc0d88fa27b4ed63ac3ff6d87c107aa6638e6169e;;
  BIN_c)  echo 9999bfac20b6c369f38103d06adb644117bc4e6b5dad286ded3dcbd524f5b70f;;
  BIN_p)  echo d920950e186ba1fbe476a804394b32fb6fe37125d33e74f8d54101b94a203153;;
  LIBC_u) echo 7a3afc14df8b7c4d64278db94a559705469ba653347a0ba711d91ddbf909967e;;
  LIBC_c) echo bf3337587f8620e477ff30c6b7c948d5b8645b3ce812642aeba9c31b4a49868b;;
  LIBC_p) echo 07c76f357231b50801e8fb24e4556108f027717f28a32dd95d53f6458c63f471;;
  *) echo unknown-pin;;
esac; }

# launch BIN ARGS... : one native run; stdout to $CUR.out, stderr to $CUR.err, the exit status in LAST_RC. The environment is cleared; cwd $RUNS; stdin /dev/null.
launch() {
  ( ulimit -c 0; ulimit -f 4096; cd "$RUNS" || exit 99
    exec env -i PATH=$CPATH HOME="$R" LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$CAP" "$@" ) < /dev/null > "$CUR.out" 2> "$CUR.err"
  LAST_RC=$?; }

# facts_run STDOUT_FILE : sets F_* from the one report line, in pure bash (no fork per line). F_LIBC_OK is 1 only for the first line starting `condlock libc=2.40 hdr=2.40 `.
facts_run() { local so=$1 line first=
  F_ROUNDS=-1 F_SIGNALS=-1 F_BLOCKED1=-1 F_MAXB=-1 F_MAXSTAG=-1 F_TICKS=-1 F_SPANNED=-1 F_NOUT=0 F_LIBC_OK=0
  [ -s "$so" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    [ -n "$line" ] || continue
    F_NOUT=$((F_NOUT + 1))
    [ "$F_NOUT" = 1 ] && first=$line
  done < "$so"
  [[ $first == "condlock libc=2.40 hdr=2.40 "* ]] && F_LIBC_OK=1
  if [[ $first =~ $RE_OUT ]]; then
    F_ROUNDS=${BASH_REMATCH[3]} F_SIGNALS=${BASH_REMATCH[4]} F_BLOCKED1=${BASH_REMATCH[5]} F_MAXB=${BASH_REMATCH[6]}
    F_MAXSTAG=${BASH_REMATCH[7]} F_TICKS=${BASH_REMATCH[8]} F_SPANNED=${BASH_REMATCH[9]}
  fi
  return 0; }

# classify_run : the repo's classify.sh on this run's exit status and files; sets C_CLASS C_SUB C_PF C_SH C_HA (class ERR if its line cannot be read)
classify_run() { local out
  out=$(sh "$CLS" "$LAST_RC" "$CUR.out" "$CUR.err" 2>&1); out=${out%%$'\n'*}
  C_CLASS=ERR C_SUB=- C_PF=-1 C_SH=-1 C_HA=-1
  if [[ $out =~ $RE_CLS ]]; then C_CLASS=${BASH_REMATCH[1]} C_SUB=${BASH_REMATCH[2]} C_PF=${BASH_REMATCH[3]} C_SH=${BASH_REMATCH[4]} C_HA=${BASH_REMATCH[6]}; fi
  return 0; }

# keep_raw : storage only, never changes a count. Every run that is not a clean NOHIT is kept (every HIT, STALL, TIMEOUT, CRASH, HARNESS-FAIL), and the first 3 clean NOHIT of each (cell, arm).
keep_raw() { local k=$1.$2
  if [ "$C_CLASS" != NOHIT ] || [ "$F_LIBC_OK" != 1 ] || [ "$F_NOUT" != 1 ] || [ "$LAST_RC" != 0 ]; then return 0; fi
  if [ "${KEPT_SAMPLE[$k]:-0}" -lt 3 ]; then KEPT_SAMPLE[$k]=$(( ${KEPT_SAMPLE[$k]:-0} + 1 )); return 0; fi
  return 1; }

# nrun ID CELL ARM REP : one native run, then one row of runs.tsv (facts and the classify.sh verdict). A HIT or STALL also gets its state dump line copied into dumps.tsv.
nrun() { local id=$1 cell=$2 arm=$3 rep=$4 args t0 t1 ms dl d=0
  local -a av
  case $arm in p) args=$ARGS_P;; *) args=$ARGS_UC;; esac
  read -r -a av <<< "$args"
  t0=${EPOCHREALTIME//[.,]/}
  launch "$G/condlock-${arm^^}" "${av[@]}"
  t1=${EPOCHREALTIME//[.,]/}; ms=$(( (t1 - t0) / 1000 ))
  facts_run "$CUR.out"; classify_run
  if [ "$C_CLASS" = HIT ] || [ "$C_CLASS" = STALL ] || [ "$C_PF" = 1 ] || [ "$C_SH" = 1 ]; then
    dl=$(grep -m 1 -E '^condlock: (HIT|STALL) ' "$CUR.err" 2> /dev/null)
    if [ -n "$dl" ]; then d=1; printf '%s\t%s\t%s\t%s\n' "$id" "$arm" "$C_CLASS" "$dl" >> "$R/dumps.tsv"; fi
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$id" "$cell" "$arm" "$rep" "$LAST_RC" "$ms" "$C_CLASS" "$C_SUB" "$C_PF" "$C_SH" "$C_HA" "$F_LIBC_OK" \
    "$F_ROUNDS" "$F_SIGNALS" "$F_BLOCKED1" "$F_MAXB" "$F_MAXSTAG" "$F_TICKS" "$F_SPANNED" "$F_NOUT" "$d" >> "$R/runs.tsv"
  if keep_raw "$cell" "$arm"; then mv "$CUR.out" "$RAW/$id.out"; mv "$CUR.err" "$RAW/$id.err"; fi
  return 0; }

# cell_np : NP_RUNS serial runs of arm P (its own setting). cell_na : rounds of U and C until the NA deadline, the starting arm alternating (round 1: U then C; round 2: C then U).
cell_np() { local i
  note "PHASE NP START arm=p runs=$NP_RUNS args=$ARGS_P"; times > "$O/times-NP-start.txt"
  for ((i = 1; i <= NP_RUNS; i++)); do
    if (( (i - 1) % OK_EVERY == 0 )); then ok_to_start || { note "PHASE NP STOPPED at run $i (see the line above)"; times > "$O/times-NP-end.txt"; return 1; }; fi
    nrun "NP.p.r$i" NP p "$i"
  done
  note "PHASE NP DONE runs=$NP_RUNS"; times > "$O/times-NP-end.txt"; return 0; }
cell_na() { local r=1 first second
  note "PHASE NA START arms=u,c args=$ARGS_UC"; times > "$O/times-NA-start.txt"
  while :; do
    [ "$EPOCHSECONDS" -ge "$ADEAD" ] && break
    if (( (r - 1) % OK_EVERY == 0 )); then ok_to_start || { note "PHASE NA STOPPED at round $r (see the line above)"; times > "$O/times-NA-end.txt"; return 1; }; fi
    if (( r % 2 == 1 )); then first=u; second=c; else first=c; second=u; fi
    nrun "NA.$first.r$r" NA "$first" "$r"; nrun "NA.$second.r$r" NA "$second" "$r"
    (( r % 200 == 0 )) && note "PROGRESS NA round=$r"
    r=$((r + 1))
  done
  note "PHASE NA DONE rounds=$((r - 1))"; times > "$O/times-NA-end.txt"; return 0; }

# gates : infrastructure and reuse checks. A gate failure is never a result (GATE-FAIL in steps.tsv, exit 5, nothing measured).
gates() { local a A
  [ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
  grep -E ' /dev/shm ' /proc/mounts | grep -q noexec && gate_fail NOEXEC
  [ "$(df -Pm "$R" | awk 'NR==2{print $4}')" -ge 1024 ] || gate_fail FREE-1GB
  taskset -c $CORES true || gate_fail CORES
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && gate_fail BUSY
  [ -d "$X1B" ] || gate_fail REUSE-DIR
  [ "$(sha_of "$X1B/cases.tsv")" = "$PIN_X1B_CASES" ] || gate_fail REUSE-RECORD-CASES
  [ "$(sha_of "$X1B/steps.tsv")" = "$PIN_X1B_STEPS" ] || gate_fail REUSE-RECORD-STEPS
  [ "$(sha_of "$X1B/ck/examples/glibc-condlock/scripts/classify.sh")" = "$PIN_CLS" ] || gate_fail REUSE-CLASSIFY
  [ "$(sha_of "$CLS")" = "$PIN_CLS" ] || gate_fail CLASSIFY-SHA
  for a in u c p; do A=${a^^}
    [ "$(sha_of "$X1B/ck/b-$A/condlock")" = "$(pin_of BIN $a)" ] || gate_fail "REUSE-BIN-$A"
    [ "$(sha_of "$X1B/$A/sysroot/usr/lib/libc.a")" = "$(pin_of LIBC $a)" ] || gate_fail "REUSE-LIBC-$A"
    cp "$X1B/ck/b-$A/condlock" "$G/condlock-$A" || gate_fail "COPY-$A"
    chmod 755 "$G/condlock-$A"
    [ "$(sha_of "$G/condlock-$A")" = "$(pin_of BIN $a)" ] || gate_fail "BIN-SHA-$A"
  done
  python3 -B "$BIN/x2_eval.py" selftest > "$O/selftest.txt" 2>&1 || gate_fail EVAL-SELFTEST
  for a in u c p; do A=${a^^}
    launch "$G/condlock-$A" -n 2 -r 1 -tick-ms 50 -q 10
    facts_run "$CUR.out"
    { [ "$LAST_RC" = 0 ] && [ "$F_LIBC_OK" = 1 ]; } || gate_fail "EXEC-$A"
  done
  note GATES-PASSED; }

# bin_check : the driver's own copies are hashed again at the end
bin_check() { local a A
  for a in u c p; do A=${a^^}; [ "$(sha_of "$G/condlock-$A")" = "$(pin_of BIN $a)" ] || note "BIN-CHANGED condlock-$A"; done; }

# ---- test seam: the Mac stub tests source this file with X2_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${X2_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "X2_TEST is set in a real run: refused" >&2; exit 5; fi
  return 0
fi

echo "$SID $T0" > "$R/sid.txt"
mkdir -p "$O" "$RUNS" "$RAW" "$R/cur"
if [ -e "$R/steps.tsv" ] || [ -e "$R/runs.tsv" ]; then echo "GATE-FAIL EXISTS: a previous run left files in $R" >&2; exit 5; fi
: > "$R/steps.tsv"; : > "$R/runs.tsv"; : > "$R/dumps.tsv"; snapshot pre
env | sort > "$O/env.txt"
gates
cell_np && cell_na
bin_check
snapshot post; leftovers > "$R/leftover.txt"; note "LEFTOVER=$(wc -l < "$R/leftover.txt") DONE"
