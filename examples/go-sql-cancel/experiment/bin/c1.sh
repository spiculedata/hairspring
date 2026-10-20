#!/usr/bin/env bash
# C1 driver: the counted runs of the sqlcancel workload under the pinned Hermit (GOC1 rev 1): one box window of `count`, or the C2a confirmation reruns of `confirm`.
# usage: setsid nohup timeout -k 30 7200 bash $R/bin/c1.sh $R count <QW> <WINNO> <MAXBLOCKS> <BLOCK_S> <CAP_S> > $R/run.out 2>&1 < /dev/null &
#        setsid nohup timeout -k 30 3600 bash $R/bin/c1.sh $R confirm <QW> <WINNO> > $R/run.out 2>&1 < /dev/null &
# The driver is uid 1000. sudo is used in exactly three places: (1) the gate `sudo -n /bin/true`; (2) the root wrapper of every Hermit call in hcore(); (3) the clean-up
# `sudo -n kill` of pids that leftover.sh listed (by hand, not here). There is no engine, no `install -d`, no root-created tree (the engine is c2e.sh's, C2 only).
# Every path Hermit writes (--summary-json, --tmp) is in a mode-1777 directory (D1). Root-owned files will remain in $R/sum (and, if Hermit writes any, in $R/tmp).
# A BLOCK is one counted index i (seed 330000+i, the same list in every arm) run in the three arms (u, c, cp) in the order that rotates with i. Cells that already have a row are
# never run again, so a stop (halt, time line, signal, kill) never changes a seed that has run; a later window asks c1_eval.py `next` for the first cell without a row.
# Nothing here decides a hit and nothing stops on a result: the halts are infrastructure only (c1_eval.py `halts`); each window's code is read before the next window starts.
# The C_* variables below are for the Mac test harness only: gate OVERRIDES refuses to run if any C_* variable is set unless C_TEST=1.
set -u
R=$1; MODE=$2; QW=$3; WINNO=$4; MAXBLOCKS=${5:-0}; BLOCK_S=${6:-0}; CAP_S=${7:-0}
T0=$(date +%s)
NEWBY=$((T0 + ${C_HARD:-6600}))            # the hard line: no new run after 110 min
HARD=$NEWBY
SOFT=$((HARD - ${C_GUARD:-180}))           # the soft line: no new block after this (3 runs x the 60 s cap still end before the hard line)
PROBE=${C_PROBE:-5}                        # the rate check runs after this many blocks of the window
# EDIT: the path of your Hermit binary (the run used the build with the sha256 below; see SCRIPTS/README.md)
HBIN=${C_HBIN:-/path/to/hermit}
HSHA=${C_HSHA:-666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a}
CE=$R/bin/c1_eval.py; GE=$R/bin/g_eval.py
G=$R/g; O=$R/out; RUNS=$R/runs; TMPS=$R/tmp; SUMD=$R/sum; PREV=$R/prev
CORES=1,2,3; EP=2026-01-01T00:00:00Z; GP=${C_GP:-/usr/bin:/bin}; CAP=${C_CAP:-60}
FS_C=(--chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot)     # flag set C (F0, design 3.1)
CELLS=()
# shellcheck source=guard.sh
. "$R/bin/guard.sh"
# shellcheck source=facts.sh
. "$R/bin/facts.sh"
echo "$SID $T0" > "$R/sid.txt"
mkdir -p "$O" "$RUNS"

pin_of() { case $1 in
  u-shim)  echo "${C_PIN_U_SHIM:-282fddca507d718fa255c3fa35c3ab6ec9d98c217f839deb80d8e48c1d6da822}";;
  c-shim)  echo "${C_PIN_C_SHIM:-6ba4c482af014bcf00b7dc175d4ed75aabc396e402a40f1a1a406910b20ca76d}";;
  cp-shim) echo "${C_PIN_CP_SHIM:-592d8435fd44f774bc18e0881b55c69dc779844c02be27603217fb3b3bc81341}";;
  *) echo "unknown-pin";;
esac; }
BINS="u-shim c-shim cp-shim"

# settle ID : after a run ended by the cap, wait up to 20 s for any process that still has $R/ in its arguments (the guest); if one is left, HALT
# (settle, hcore and hrun below are byte-identical to g.sh's; g.sh happens to use the name i elsewhere, so only here shellcheck needs the next line)
# shellcheck disable=SC2034
settle() { local i
  for i in $(seq 1 20); do procs_with_rdir > "$R/procs-now.txt"; [ -s "$R/procs-now.txt" ] || return 0; sleep 1; done
  echo "$1 rc=$H_RC" > "$R/HALT"; note "TIMEOUT-HALT $1 rc=$H_RC: a process still has $R/ in its arguments after 20 s; no further step; root processes may be left (section 11)"; return 0; }

# hcore ID SEED CMD... : one Hermit run under the root wrapper (the D1 / F0 / F0-e wrapper). Sets H_RC and H_MS. Writes $O/ID.{stdout,stderr,rc}; the summary goes to $SUMD/ID.summary.json.
hcore() { local id=$1 seed=$2 t0 t1; shift 2
  mkdir -p "$RUNS/$id" "$TMPS/$id"; chmod 1777 "$TMPS/$id"
  t0=${EPOCHREALTIME//[.,]/}
  ( ulimit -c 0; ulimit -f 4096; cd "$RUNS/$id" || exit 99
    sudo -n timeout -k 10 "$CAP" env -i PATH=$GP HOME=/root LANG=C nice -n 10 taskset -c $CORES \
      "$HBIN" run --epoch=$EP --base-env=minimal --env=GOMAXPROCS=2 --env=GOTRACEBACK=all "${FS_C[@]}" \
      --sched-seed="$seed" --tmp="$TMPS/$id" --summary-json="$SUMD/$id.summary.json" -- "$@" ) < /dev/null > "$O/$id.stdout" 2> "$O/$id.stderr"
  H_RC=$?; t1=${EPOCHREALTIME//[.,]/}; H_MS=$(( (t1 - t0) / 1000 )); echo "$H_RC" > "$O/$id.rc"
  if [ "$H_RC" = 124 ] || [ "$H_RC" = 137 ]; then settle "$id"; fi
  return 0; }

# hrun ID STEP ARM BUILD MODE SEED REP Q : one sqlcancel run under Hermit, then one row of runs.tsv (facts only; classes are assigned by sclib.py)
hrun() { local id=$1 step=$2 arm=$3 build=$4 mode=$5 seed=$6 rep=$7 q=$8 bin fpx fp sc ssha esha
  ok_to_start || return 1
  case $build in s) bin=$G/$arm-shim;; *) bin=$G/$arm-vo;; esac
  hcore "$id" "$seed" "$bin" -mode="$mode" -q "$q"
  ssha=$(sha256sum < "$O/$id.stdout" | cut -c1-16)
  esha=$(grep -E '^HAIRSPRING_(EVENT|VIOLATION)' "$O/$id.stderr" | sha256sum | cut -c1-16)
  fpx="NOSUMMARY NA"; [ -s "$SUMD/$id.summary.json" ] && fpx=$(python3 -B "$R/bin/fpx.py" "$SUMD/$id.summary.json")
  read -r fp sc <<< "$fpx"
  facts_row "$O/$id.stdout" "$O/$id.stderr"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t' "$id" "$step" "$arm" "$build" "$mode" "$seed" "$rep" "$H_RC" "$H_MS" "$ssha" "$esha" "$fp" "$sc" >> "$R/runs.tsv"
  facts_tsv >> "$R/runs.tsv"
  return 0; }

# run_cells : run the cells that c1_eval.py `next` named (the lines of CELLS: "RUN C1 <arm> <seed> <idx>"), in that order
run_cells() { local line step arm seed idx id
  for line in "${CELLS[@]}"; do
    read -r _ step arm seed idx <<< "$line"
    id=$(printf 'C1.%s.i%04d' "$arm" "$idx")
    hrun "$id" "$step" "$arm" s std "$seed" 1 "$QW" || return 1
  done
  return 0; }

# halt_rules : the pre-registered infrastructure halts over this window's rows; a halt ends the window after the block that tripped it (return 2)
halt_rules() { local hc
  hc=$(python3 -B "$CE" halts "$R/runs.tsv" 2>> "$O/gate-errors.txt")
  case $hc in
    OK) return 0;;
    "HALT "*) note "HALT-RULE ${hc#HALT }"; return 2;;
    *) note "HALT-RULE C1-HALT/EVAL-ERROR ${hc:-NO-VERDICT}"; return 2;;
  esac; }

# step_count : blocks until MAXBLOCKS, the soft line, a refusal, a halt rule, a pause, or every index is decided. Return 0 normal end, 1 refused (guard.sh wrote the reason), 2 halt rule, 3 pause.
step_count() { local nb=0 max=$MAXBLOCKS probed=0 v tb0
  tb0=$(date +%s)
  while :; do
    NEWBY=$SOFT; ok_to_start || return 1
    mapfile -t CELLS < <(python3 -B "$CE" next "$R/runs.tsv" 2>> "$O/gate-errors.txt")
    case ${CELLS[0]:-NO-VERDICT} in
      DONE) note "C1-COMPLETE every index is decided in every arm after $nb blocks in this window"; return 0;;
      "RUN C1 "*) if [ "$nb" -ge "$max" ]; then note "WINDOW-FULL blocks=$nb"; return 0; fi;;
      *) note "HALT-RULE C1-HALT/EVAL-ERROR ${CELLS[0]:-NO-VERDICT}"; return 2;;
    esac
    NEWBY=$HARD
    run_cells || return 1
    nb=$((nb + 1))
    halt_rules || return 2
    if [ "$probed" = 0 ] && [ "$nb" -ge "$PROBE" ]; then
      probed=1
      v=$(python3 -B "$CE" probe "$BLOCK_S" "$PROBE" "$(( $(date +%s) - tb0 ))" 2>> "$O/gate-errors.txt")
      note "PROBE ${v:-NO-VERDICT}"
      case $v in
        OK*) ;;
        *) note "PAUSE-BUDGET C1-PAUSE/RATE ${v:-NO-VERDICT}: the observed time per block is not the plan's; no further block; the plan is re-derived under a reviewed revision"; return 3;;
      esac
    fi
  done; }

# step_confirm : C2D, 3 more runs of every A-u hit seed (up to the cap) on the same binary and flags, from the earlier windows' rows
step_confirm() { local s r
  mapfile -t CELLS < <(python3 -B "$CE" hits "$PREV" 2>> "$O/gate-errors.txt")
  case ${CELLS[0]:-NO-VERDICT} in
    NONE) note "C2D-COMPLETE no A-u hit in the earlier windows; nothing to confirm"; return 0;;
    "HITS "*) note "C2D ${CELLS[0]}";;
    *) note "HALT-RULE C1-HALT/EVAL-ERROR ${CELLS[0]:-NO-VERDICT}"; return 2;;
  esac
  NEWBY=$HARD
  for s in "${CELLS[@]:1}"; do
    for r in 1 2 3; do hrun "C2D.u.s$s.r$r" C2D u s std "$s" "$r" "$QW" || return 1; done
    halt_rules || return 2
  done
  note "C2D-COMPLETE every listed hit seed has its 3 reruns"
  return 0; }

if [ -e "$R/steps.tsv" ] || [ -e "$R/runs.tsv" ]; then echo "GATE-FAIL EXISTS: a previous run left files in $R" >&2; exit 5; fi
: > "$R/steps.tsv"; : > "$R/runs.tsv"; snapshot pre
env | sort > "$O/env.txt"
# GATES (infrastructure; a gate failure is never a result)
[ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
if env | grep -q '^C_' && [ "${C_TEST:-0}" != 1 ]; then env | grep '^C_' > "$O/overrides.txt"; gate_fail OVERRIDES; fi
case $MODE in count|confirm) ;; *) gate_fail ARGS;; esac
case $QW in 500|250) ;; *) gate_fail ARGS;; esac
{ [[ $WINNO =~ ^[0-9]+$ ]] && [ "$WINNO" -ge 1 ] && [ "$WINNO" -le 20 ]; } || gate_fail ARGS
if [ "$MODE" = count ]; then
  { [[ $MAXBLOCKS =~ ^[0-9]+$ ]] && [[ $BLOCK_S =~ ^[0-9]+$ ]] && [[ $CAP_S =~ ^[0-9]+$ ]] \
    && [ "$MAXBLOCKS" -ge 1 ] && [ "$MAXBLOCKS" -le 2000 ] && [ "$BLOCK_S" -ge 1 ] && [ "$CAP_S" -ge 3600 ]; } || gate_fail ARGS
fi
note "ARGS mode=$MODE qw=$QW win=$WINNO maxblocks=$MAXBLOCKS block_s=$BLOCK_S cap_s=$CAP_S"; echo "$MODE $QW $WINNO $MAXBLOCKS $BLOCK_S $CAP_S" > "$O/args.txt"
sudo -n /bin/true > /dev/null 2>&1 || gate_fail SUDO
grep -E ' /dev/shm ' /proc/mounts | grep -q noexec && gate_fail NOEXEC
[ "$(df -Pm "$R" | awk 'NR==2{print $4}')" -ge 1024 ] || gate_fail FREE-1GB
for pth in "$R" "$R/g"; do other_has 5 "$pth" || gate_fail PERMS; done
for b in $BINS; do other_has 5 "$G/$b" || gate_fail PERMS; done
for pth in "$SUMD" "$TMPS"; do [ "$(stat -c %a "$pth")" = 1777 ] || gate_fail PERMS-1777; done
pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && gate_fail BUSY
[ "$(sha256sum "$HBIN" | cut -d' ' -f1)" = "$HSHA" ] || gate_fail HERMIT-SHA
for b in $BINS; do [ "$(sha256sum "$G/$b" | cut -d' ' -f1)" = "$(pin_of "$b")" ] || gate_fail "BIN-SHA-$b"; done
python3 -B "$GE" selftest > "$O/selftest-g.txt" 2>&1 || gate_fail EVAL-SELFTEST
python3 -B "$CE" selftest > "$O/selftest-c1.txt" 2>&1 || gate_fail EVAL-SELFTEST
if [ "$WINNO" -eq 1 ] && [ "$MODE" = count ]; then
  [ ! -e "$PREV" ] || gate_fail PREV-UNEXPECTED
else
  v=$(python3 -B "$CE" prevcheck "$PREV" "$QW" 2>> "$O/gate-errors.txt"); note "PREV-CHECK ${v:-NO-VERDICT}"
  case $v in OK*) ;; *) gate_fail PREV-BAD;; esac
fi
hcore GATE-H 330000 /bin/echo smoke
{ [ "$H_RC" = 0 ] && [ "$(cat "$O/GATE-H.stdout")" = smoke ]; } || gate_fail HERMIT-SMOKE
[ "$(python3 -B "$R/bin/fpx.py" "$SUMD/GATE-H.summary.json" | cut -d' ' -f1)" != NOSUMMARY ] || gate_fail HERMIT-SUMMARY
hcore GATE-S 330000 "$G/u-shim" -mode=std -q 4
facts_row "$O/GATE-S.stdout" "$O/GATE-S.stderr"
{ [ "$H_RC" = 0 ] && [ "$F_Q" = 16 ] && [ "$F_NEV" = 17 ] && [ "$F_END" = 0 ]; } || gate_fail HERMIT-EVENTS
note "GATES-PASSED"

# THE WINDOW
RC=0
if [ "$MODE" = count ]; then
  v=$(python3 -B "$CE" pausehours "$PREV" "$BLOCK_S" "$CAP_S" "$MAXBLOCKS" 2>> "$O/gate-errors.txt")
  note "PAUSE-CHECK hours ${v:-NO-VERDICT}"
  case $v in
    OK*) step_count; RC=$?;;
    *) note "PAUSE-BUDGET C1-PAUSE/HOURS ${v:-NO-VERDICT}: no run is started; the total cap is reached or would be passed; C1 reads R-INCONC (what ran and why) unless the cap is raised; the seed list never changes"; RC=3;;
  esac
else
  step_confirm; RC=$?
fi
note "WINDOW-RC $RC"
for b in $BINS; do [ "$(sha256sum "$G/$b" | cut -d' ' -f1)" = "$(pin_of "$b")" ] || note "BIN-CHANGED $b"; done
snapshot post; leftovers > "$R/leftover.txt"
note "WINDOW-WALL $(( $(date +%s) - T0 ))"; note "LEFTOVER=$(wc -l < "$R/leftover.txt") DONE"
