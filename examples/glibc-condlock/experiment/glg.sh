#!/usr/bin/env bash
# glg.sh : GLG driver, the Hermit gates of the glibc `condlock` workload under the pinned Hermit, in TWO parts selected by the 4th argument:
#   PART A (does not depend on P): T-probe recheck on arm C, G0 with its triples and the G0d/G0c/G0w/G0t reads for A-u and A-c (this freezes RND), the projected-time check,
#     D-pos-H, D-stall-H, G2a/b. No P binary, no P pin and no P file is read.
#   PART P (after the P2 tree exists; reuses the RND that PART A froze): T-probe recheck on arm P, the P-u determinism triples, G0w and G0t for P-u, the projected-time check, G1, G2c.
#     Its pins come from $BIN/glg-p2.sh (a separate file, never filled: PART P was never run); nothing of the first P tree is pinned anywhere.
# uid 1000; sudo only for the Hermit wrapper (see below).
# usage: taskset -c 0 setsid nohup timeout -k 30 <3600 for A, 7200 for P> bash $R/bin/glg.sh $R <RND> <G0S> <A|P> > $R/run.out 2>&1 < /dev/null &
# RND is the design's R (rounds per worker, the workload's -r flag); <RND> <G0S> must be one row of the launch table: 500 319101, 250 319511, 125 319521, 60 319531, 30 319541.
# sudo is used in exactly three places: (1) the gate `sudo -n /bin/true`; (2) the root wrapper of every Hermit call in hcore() (timeout INSIDE sudo; the stdout and stderr
# redirects stay outside, so the logs are ubuntu-owned); (3) the clean-up `sudo -n kill` of pids that leftover.sh listed (by hand, not here). Every path Hermit writes
# (--summary-json, --tmp) is in a mode-1777 directory (D1). Root-owned files will remain in $R/sum and $R/tmp.
# Builds: none. The planted and violation-only binaries (21 of X1c's 22 are used: PART A) are built, isolation-checked and pinned by the separate X1c spec; here they are copied from the X1c work dir after their
# sha256 are checked against the pin table PLB_PINS below (filled from X1c's pin-table.tsv; the gate PIN-TABLE stops the job unless all are 64 hex digits). Nothing is compiled or installed.
# The steps run in order and each gate is evaluated by glg_eval.py (the one implementation): a STOP or MISSING ends the chain and the later steps are NOT-RUN. Nothing stops on a
# result inside a step (no early stopping). The two exceptions are in G0: the ALL-BAD skip after the first seed, and the SKID_OVERSHOOT replacement of a pair by a spare seed.
set -u
R=$1; RND=${2:-}; G0S=${3:-}; PART=${4:-}
T0=$(date +%s)
if [ "$PART" = P ]; then NEWBY=$((T0 + 110 * 60)); else NEWBY=$((T0 + 50 * 60)); fi   # no new unit of work after 110 min (P, cap 120 min) or 50 min (A, cap 60 min)
BIN=$R/bin; G=$R/g; O=$R/out; RUNS=$R/runs; TMPS=$R/tmp; SUMD=$R/sum
CUR=$R/cur/c; CLS=$BIN/classify.sh; GE=$BIN/glg_eval.py
CORES=1,2,3; HCORE=1; CPATH=/usr/bin:/bin; CAP=120; NCAP=60
HBIN=/path/to/hermit   # EDIT: the pinned hermit binary (its sha256 must equal HSHA below, gate HERMIT-SHA)
HSHA=666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a
EP=2026-01-01T00:00:00Z
FS_C=(--chaos --max-timeslice=100000 --skid-margin=25 --panic-on-rbc-overshoot)     # flag set C (design 7.1; X1b)
TICK=100; QUORUM=10; NWORK=8                                                          # virtual tick 100 ms, Q = 10, N = 8 (design 2.4, 2.1)
KS=(1 2 5 20 50)                                                                      # the plant k list (design 6.1), cycled over the seeds
TSEED=319701; SMOKE_SEED=319001
G1_FIRST=319201; G1_N=200; G2A_FIRST=319401; G2A_N=10; DP_FIRST=319601; DP_N=30; DS_N=10
# Pins (spec section 3). The X1b work dir is only read: sha256sum of the files below, cp of the two condlock binaries and the C tprobe (the SDK archive is only hashed), no compiler runs.
X1B=/path/to/x1b-glibc-20261017T1441Z   # EDIT: the work dir of x1b.sh (stamp 20261017T1441Z in the published run)
PIN_CLS=5fbc06df3de905c7c67a745d1b8c93c62d886c182f98ac85f93c92433637d32a
PIN_X1B_CASES=0fd5a4a8f34c550d4e81d1d890ae1fa3fb40918e8f98bd02059f79a37e082eb1
PIN_X1B_STEPS=ea10be71b3e814b697d13db4068734cf7dfb805e06bb1b0b18b9093e0b485604
# ---- PIN TABLE FROM X1c (X1c ran, stamp 20261017T2115Z, X1C-PASS+CTRL-DIFFER-UCP). The 22 values are X1c's pin-table.tsv (table sha256
# 0fb6896870a547fe0f4008c566ecd817319219dbd3d18946ee0c9f46ab684105), re-hashed from the archive's pin/ directory. The gate PIN-TABLE refuses to start unless every value is 64 hex digits.
X1C=/path/to/x1c-glibc-20261017T2115Z          # EDIT: the work dir of x1c.sh; the pinned binaries are at $X1C/pin/<name>
PIN_X1C_CASES=1063e39112047b08473e3f32a9d48bcfe0eb0c6b64d20ce7a396b645d9745b94   # sha256 of X1c's cases.tsv (archive copy; equal to the box original by [I])
PIN_X1C_STEPS=5905f05419e7f1c6caf65a425bba233b0985351c2bd17a5cf9a437844dec3df8   # sha256 of X1c's steps.tsv (last line DONE-OVERALL; archive copy, same [I])
declare -A PLB_PINS=(
  [condlock-drop-k1]=fa23d2873b1c0a2875c0b2a7afbf0c99fe7e850535e213a4fe29e25a2be997bb [condlock-drop-k2]=2d56e3ac922e30d76f7ee9f12a43a408739a57670e1bd569c0a302e285094883
  [condlock-drop-k5]=48f3c80c0eb5471e0bf18dfb6923fd74239387fd19b56d9d4c68250115ca38b2 [condlock-drop-k20]=8e88e40677e6bb297d9d665d92a1bbf56583ee76b2ac0947cd06576de45f36de
  [condlock-drop-k50]=0e02bdbe6e538072ed47a1808c6fbb5c6800673bd70abbe13b07d3f093dbba63
  [condlock-leak-k1]=be36380b361b7aa47f86168d82ccf5c136adaa2878a6c38fb3b722cd1a6f4560 [condlock-leak-k2]=9a44736227b2b3470e4eaaeec963d546b55ec28a23e8c3c309bfe412d17fd89d
  [condlock-leak-k5]=3107289cf65954c6196f63a79f37070ce22c4d7fb27abead55998b332e7b9405 [condlock-leak-k20]=52d8b82e56e9cf871dd826d8460b964ae3df167486377d35f2a25ff086bdd0b7
  [condlock-leak-k50]=6cf657741b444c0a0e076af8577810f158987a4d22c5baf89daa3529ebef712d
  [condlock-barrier-k1]=83e25993838adf2148adeba53272b5efd28adab4486b39b9d8ea46895ad3e57c [condlock-barrier-k2]=a0513484f60b74cfcd6b94c3570ae23fa209948de082d0cf02c16f42a8bc8f2b
  [condlock-barrier-k5]=72b6a32a10b53a779e04b5f66e45ec9cbe70559bac2f52c979b694dcecb6c89e [condlock-barrier-k20]=d234f52e589b7b2a040224c9603d14fe8980936dc315e6720c991ab09b7dbbbf
  [condlock-barrier-k50]=d87836acf05bc5d7a5cc56574cf377bda58bd0f500195c80a102b837f339297d
  [condlock-holdg-k1]=fb7d161fd3629a46d53512422b1a6ea241226d979ad10846cdb0309231158c21 [condlock-holdg-k2]=296f0451d9edcf9a36d1fe50904fd8b96006fac7fa0383b99df8990102da54f2
  [condlock-holdg-k5]=31b82eacc1b851d3cec64f630b66be3214a799a1603e672335df5b4df210a73d [condlock-holdg-k20]=83bb9995f3d4abf7c4e8bec5ec7fc016161c098823251dd17d7c14831ef359c9
  [condlock-holdg-k50]=bae88d393bf534e035f1a09bfcc0ab6cf8eaa34bbc6b01171a27d52a681522a5
  [condlock-novio-U]=2d3f436f4bca87d4a177197f98a0e0363f2ed3085b83705f1bf43c13bfb163fa)
# (X1c also pinned a violation-only build for the first P tree, condlock-novio-P. It is NOT pinned here: it is P-dependent, and the first P tree is not used.)
PLB_NAMES=(condlock-drop-k1 condlock-drop-k2 condlock-drop-k5 condlock-drop-k20 condlock-drop-k50 condlock-leak-k1 condlock-leak-k2 condlock-leak-k5 condlock-leak-k20 condlock-leak-k50
  condlock-barrier-k1 condlock-barrier-k2 condlock-barrier-k5 condlock-barrier-k20 condlock-barrier-k50 condlock-holdg-k1 condlock-holdg-k2 condlock-holdg-k5 condlock-holdg-k20 condlock-holdg-k50
  condlock-novio-U)
# ---- PART P only: the pins of the P2 tree come from $BIN/glg-p2.sh (never filled; PART P was never run). The defaults below are empty so that PART A never reads or needs them.
P2F=$BIN/glg-p2.sh
P2_DIR=; P2_PIN_CASES=; P2_PIN_STEPS=; P2_PIN_BIN=; P2_PIN_TPROBE=; P2_PIN_NOVIO=
# shellcheck source=guard.sh
. "$R/bin/guard.sh"
# shellcheck source=/dev/null
if [ "$PART" = P ] && [ -r "$P2F" ]; then . "$P2F"; fi

RE_OUT='^condlock libc=([^ ]+) hdr=([^ ]+) rounds= *([0-9]+) signals= *([0-9]+) blocked1= *([0-9]+) maxb= *([0-9]+) maxstag= *([0-9]+) ticks= *([0-9]+) spanned= *([0-9]+)( applied=([01]))?$'
RE_CLS='^class=([A-Z-]+) subtype=([A-Za-z+-]+) prop_false=([01]) stall_hit=([01]) applied=([01-]) hit_also=([01]) detail='

sha_of() { sha256sum "$1" 2> /dev/null | cut -d' ' -f1; }
# pin_of BIN|LIBC|TPROBE|SDK u|c : the X1b pins of arms U and C only (no P pin: the first P tree is not pinned; binary = X1b binaries.sha; libc.a = X1b cases.tsv B-libc-<arm>; tprobe and SDK = the X1b build, hashed in its archive)
pin_of() { case "$1_$2" in
  BIN_u)  echo 9a34111a27d44d28b580b09cc0d88fa27b4ed63ac3ff6d87c107aa6638e6169e;;
  BIN_c)  echo 9999bfac20b6c369f38103d06adb644117bc4e6b5dad286ded3dcbd524f5b70f;;
  LIBC_u) echo 7a3afc14df8b7c4d64278db94a559705469ba653347a0ba711d91ddbf909967e;;
  LIBC_c) echo bf3337587f8620e477ff30c6b7c948d5b8645b3ce812642aeba9c31b4a49868b;;
  TPROBE_c) echo 11a64c075a35b124fb7f63d63805bee3ce9f11c9a256bee874179394c931dcdf;;
  SDK_c)  echo 3b28d4032a296565a8bd3b6f7ffe6978a5c69949085c62e9a92b19f808617cec;;
  *) echo unknown-pin;;
esac; }
jnum() { sed -n "s/^ *\"$2\": *\([0-9][0-9]*\).*/\1/p" "$1" 2> /dev/null | head -n 1; }   # the first integer after "key": in a Hermit summary

# launch BIN ARGS... : one NATIVE run (gates only); stdout to $CUR.out, stderr to $CUR.err, the exit status in LAST_RC. The environment is cleared; cwd $RUNS; stdin /dev/null.
launch() {
  ( ulimit -c 0; ulimit -f 4096; cd "$RUNS" || exit 99
    exec env -i PATH=$CPATH HOME="$R" LANG=C LC_ALL=C nice -n 10 taskset -c $CORES timeout -k 5 "$NCAP" "$@" ) < /dev/null > "$CUR.out" 2> "$CUR.err"
  LAST_RC=$?; }

# facts_run STDOUT_FILE : sets F_* from the one report line, in pure bash (no fork per line). F_LIBC_OK is 1 only for the first line starting `condlock libc=2.40 hdr=2.40 `.
# The planted build's optional ` applied=` field is accepted by RE_OUT; its value is read by classify.sh (C_AP), not here.
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

# site_trues STDERR_FILE SITE_ID : the `trues` counter of that assert line of the SDK report, or -1 (violation-only builds have none)
site_trues() { local line v=-1 re='"trues": *([0-9]+)'
  line=$(grep -F "\"id\":\"$2\"" "$1" 2> /dev/null | grep -F '"kind":"assert"' | head -n 1)
  if [[ $line =~ $re ]]; then v=${BASH_REMATCH[1]}; fi
  echo "$v"; }

# classify_run RC OUT ERR : the repo's classify.sh; sets C_CLASS C_SUB C_PF C_SH C_AP C_HA (class ERR if its line cannot be read)
classify_run() { local out
  out=$(sh "$CLS" "$1" "$2" "$3" 2>&1); out=${out%%$'\n'*}
  C_CLASS=ERR C_SUB=- C_PF=-1 C_SH=-1 C_AP=- C_HA=-1
  if [[ $out =~ $RE_CLS ]]; then C_CLASS=${BASH_REMATCH[1]} C_SUB=${BASH_REMATCH[2]} C_PF=${BASH_REMATCH[3]} C_SH=${BASH_REMATCH[4]} C_AP=${BASH_REMATCH[5]} C_HA=${BASH_REMATCH[6]}; fi
  return 0; }

# settle ID : after a run ended by the cap, wait up to SETTLE_TRIES seconds (20) for any process that still has $R/ in its arguments (the guest); if one is left, HALT
SETTLE_TRIES=20
settle() { local i
  for i in $(seq 1 "$SETTLE_TRIES"); do procs_with_rdir > "$R/procs-now.txt"; [ -s "$R/procs-now.txt" ] || return 0; sleep 1; done
  echo "$1 rc=$H_RC" > "$R/HALT"; note "TIMEOUT-HALT $1 rc=$H_RC: a process still has $R/ in its arguments after 20 s; no further step; root processes may be left (section 12)"; return 0; }

# hcore ID SEED CMD... : one Hermit run under the root wrapper (the D1 / F0 / X1b wrapper). Sets H_RC and H_MS. Writes $O/ID.{stdout,stderr,rc}; the summary goes to $SUMD/ID.summary.json.
hcore() { local id=$1 seed=$2 t0 t1; shift 2
  mkdir -p "$RUNS/$id" "$TMPS/$id"; chmod 1777 "$TMPS/$id"
  t0=${EPOCHREALTIME//[.,]/}
  ( ulimit -c 0; ulimit -f 4096; cd "$RUNS/$id" || exit 99
    sudo -n timeout -k 10 "$CAP" env -i PATH=$CPATH HOME=/root LANG=C nice -n 10 taskset -c $HCORE \
      "$HBIN" run --epoch=$EP --base-env=minimal "${FS_C[@]}" \
      --sched-seed="$seed" --tmp="$TMPS/$id" --summary-json="$SUMD/$id.summary.json" -- "$@" ) < /dev/null > "$O/$id.stdout" 2> "$O/$id.stderr"
  H_RC=$?; t1=${EPOCHREALTIME//[.,]/}; H_MS=$(( (t1 - t0) / 1000 )); echo "$H_RC" > "$O/$id.rc"
  if [ "$H_RC" = 124 ] || [ "$H_RC" = 137 ]; then settle "$id"; fi
  return 0; }

# hrun ID STEP ARM BIN SEED REP K ARGS... : one condlock run under Hermit, then one row of runs.tsv (31 columns, the schema of glg_eval.py COLS). BIN is a file name under $G.
# A HIT or STALL also gets its state dump line copied into dumps.tsv.
hrun() { local id=$1 step=$2 arm=$3 bin=$4 seed=$5 rep=$6 k=$7 ssha fpx fp sc virt turns dl d=0 ct wt; shift 7
  ok_to_start || return 1
  hcore "$id" "$seed" "$G/$bin" "$@"
  facts_run "$O/$id.stdout"; classify_run "$H_RC" "$O/$id.stdout" "$O/$id.stderr"
  ssha=$(sha256sum < "$O/$id.stdout" | cut -c1-16)
  fpx="NOSUMMARY NA"; [ -s "$SUMD/$id.summary.json" ] && fpx=$(python3 -B "$BIN/fpx.py" "$SUMD/$id.summary.json")
  read -r fp sc <<< "$fpx"
  virt=$(jnum "$SUMD/$id.summary.json" virttime_elapsed); turns=$(jnum "$SUMD/$id.summary.json" sched_turns)
  ct=$(site_trues "$O/$id.stderr" condlock.contended_signal); wt=$(site_trues "$O/$id.stderr" condlock.wait_spanned_two_signals)
  if [ "$C_CLASS" = HIT ] || [ "$C_CLASS" = STALL ] || [ "$C_PF" = 1 ] || [ "$C_SH" = 1 ]; then
    dl=$(grep -m 1 -E '^condlock: (HIT|STALL) ' "$O/$id.stderr" 2> /dev/null)
    if [ -n "$dl" ]; then d=1; printf '%s\t%s\t%s\t%s\n' "$id" "$arm" "$C_CLASS" "$dl" >> "$R/dumps.tsv"; fi
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$id" "$step" "$arm" "$k" "$seed" "$rep" "$H_RC" "$H_MS" "$C_CLASS" "$C_SUB" "$C_PF" "$C_SH" "$C_AP" "$C_HA" "$F_LIBC_OK" \
    "$F_ROUNDS" "$F_SIGNALS" "$F_BLOCKED1" "$F_MAXB" "$F_MAXSTAG" "$F_TICKS" "$F_SPANNED" "$F_NOUT" "$ct" "$wt" "$ssha" "$fp" "${sc:-NA}" "${virt:--1}" "${turns:--1}" "$d" >> "$R/runs.tsv"
  return 0; }

# tp_run ID CASE ARM BIN : one tprobe run under Hermit (the T-probe recheck), then one row of tp.tsv (id case arm rc ms virt_ns turns ok)
tp_run() { local id=$1 c=$2 arm=$3 bin=$4 ok=0 virt turns want
  ok_to_start || return 1
  want="tprobe $c ok slept=60 woke=1"
  hcore "$id" "$TSEED" "$G/$bin" "$c" 60
  if [ "$H_RC" = 0 ] && [ "$(cat "$O/$id.stdout")" = "$want" ] && ! grep -qE 'Deadlock detected|panicked|SKID_OVERSHOOT|HERMIT_INTERNAL_FAILURE' "$O/$id.stderr"; then ok=1; fi
  virt=$(jnum "$SUMD/$id.summary.json" virttime_elapsed); turns=$(jnum "$SUMD/$id.summary.json" sched_turns)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$c" "$arm" "$H_RC" "$H_MS" "${virt:--1}" "${turns:--1}" "$ok" >> "$R/tp.tsv"
  return 0; }

# gate_step NAME : the pre-registered gate of glg_eval.py after a step; proceeds only on PASS
gate_step() { local v
  v=$(python3 -B "$GE" gate "$1" "$R/runs.tsv" "$RND" "$G0S" "$PART" 2>> "$O/gate-errors.txt"); note "GATE-READ $1 ${v:-NO-VERDICT}"
  case $v in PASS*) return 0;; *) return 1;; esac; }

# pause_ok : the projected-time check after G0 (design 7.4), per part. PART A: 80 x (the larger of the A-u and A-c medians) + 600 s (D-pos-H 30, D-stall-H 30, G2a 20 runs);
# PART P: 400 x (P-u median) + 600 s (G1 200, G2c 200 runs). Both against the time left before the no-new-runs line.
# Over: record PAUSE-BUDGET (a stop, not a failure; the next steps are not started) and end the chain.
pause_ok() { local v nxt
  v=$(python3 -B "$GE" pause "$R/runs.tsv" "$G0S" "$(date +%s)" "$NEWBY" "$PART" 2>> "$O/gate-errors.txt"); note "PAUSE-CHECK ${v:-NO-VERDICT}"
  if [ "$PART" = P ]; then nxt=G1; else nxt=D-pos-H; fi
  case $v in OK*) return 0;; *) note "PAUSE-BUDGET ${v:-NO-VERDICT}: $nxt is not started; a later launch starts at $nxt with RND=$RND and the G0 block in force, under a separately reviewed revision"; return 1;; esac; }

# ---- steps (each runs all its runs; the gate is read afterwards)
WARGS() { echo "-n $NWORK -r $RND -tick-ms $TICK -q $QUORUM"; }
# the arms of the part (G0 and the first-seed test), the arm of its T-probe recheck, and the binary names (the P arm is the P2 tree's binary; nothing of the first P tree exists here)
part_arms() { if [ "$PART" = P ]; then echo p; else echo "u c"; fi; }
tp_arm() { if [ "$PART" = P ]; then echo p; else echo c; fi; }
bin_of() { case $1 in u) echo condlock-U;; c) echo condlock-C;; p) echo condlock-P2;; un) echo condlock-novio-U;; pn) echo condlock-novio-P2;; *) echo "condlock-unknown-arm-$1";; esac; }
tbin_of() { case $1 in c) echo tprobe-C;; p) echo tprobe-P2;; *) echo "tprobe-unknown-arm-$1";; esac; }
step_tp() { local a c
  : > "$R/tp.tsv"
  a=$(tp_arm); for c in i iii; do tp_run "TP.$c.$a" "$c" "$a" "$(tbin_of "$a")" || return 1; done
  return 0; }
# g0_replace ARM SEED : the pre-registered SKID_OVERSHOOT rule. glg_eval.py says "RUN <spare>" when the pair just run is to be replaced and the spare's runs are not yet made:
# the 3 runs of the same arm at the spare seed (step G0R) follow, and a spare that fails the same way is asked about again. At most 3 rounds, belt and braces.
g0_replace() { local a=$1 s=$2 v sp r n=0 av
  read -r -a av <<< "$(WARGS)"
  while [ "$n" -lt 3 ]; do
    n=$((n + 1))
    v=$(python3 -B "$GE" skidnext "$R/runs.tsv" "$a" "$s" "$G0S" "$PART" 2>> "$O/gate-errors.txt")
    case $v in
      "RUN "[0-9]*) sp=${v#RUN }; note "SKID-REPLACE $a seed $s -> spare seed $sp (glg_eval.py skidnext)"
        for r in 1 2 3; do hrun "G0R.$a.s$sp.r$r" G0R "$a" "$(bin_of "$a")" "$sp" "$r" 0 "${av[@]}" || return 1; done;;
      *) return 0;;
    esac
  done
  return 0; }
step_g0() { local s a r i=0 v av na
  read -r -a av <<< "$(WARGS)"
  na=$(part_arms | wc -w | tr -d ' ')
  for s in $(seq "$G0S" $((G0S + 9))); do
    for a in $(part_arms); do
      for r in 1 2 3; do hrun "G0.$a.s$s.r$r" G0 "$a" "$(bin_of "$a")" "$s" "$r" 0 "${av[@]}" || return 1; done
      g0_replace "$a" "$s" || return 1
    done
    i=$((i + 1))
    if [ "$i" = 1 ]; then
      v=$(python3 -B "$GE" first "$R/runs.tsv" "$G0S" "$PART" 2> /dev/null)
      if [ "$v" = ALLBAD ]; then note "G0 ALL-BAD after the first seed: all $((3 * na)) runs failed to complete; the rest of G0 is skipped (F0 precedent)"; return 0; fi
    fi
  done
  return 0; }
step_dposh() { local i s k av
  read -r -a av <<< "$(WARGS)"
  for ((i = 0; i < DP_N; i++)); do s=$((DP_FIRST + i)); k=${KS[$((i % 5))]}
    hrun "DPH.s$s" DPH d "condlock-drop-k$k" "$s" 1 "$k" "${av[@]}" || return 1; done
  return 0; }
step_dstallh() { local spec plant arm first i s k av
  read -r -a av <<< "$(WARGS)"
  for spec in leak:l:319641 barrier:b:319651 holdg:h:319661; do
    IFS=: read -r plant arm first <<< "$spec"
    for ((i = 0; i < DS_N; i++)); do s=$((first + i)); k=${KS[$((i % 5))]}
      hrun "DSH.$arm.s$s" DSH "$arm" "condlock-$plant-k$k" "$s" 1 "$k" "${av[@]}" || return 1; done
  done
  return 0; }
step_g1() { local i s av
  read -r -a av <<< "$(WARGS)"
  for ((i = 0; i < G1_N; i++)); do s=$((G1_FIRST + i)); hrun "G1.p.s$s" G1 p "$(bin_of p)" "$s" 1 0 "${av[@]}" || return 1; done
  return 0; }
step_g2a() { local i s av
  read -r -a av <<< "$(WARGS)"
  for ((i = 0; i < G2A_N; i++)); do s=$((G2A_FIRST + i))
    hrun "G2A.u.s$s" G2A u condlock-U "$s" 1 0 "${av[@]}" || return 1
    hrun "G2A.un.s$s" G2A un condlock-novio-U "$s" 1 0 "${av[@]}" || return 1; done
  return 0; }
step_g2c() { local i s av
  read -r -a av <<< "$(WARGS)"
  for ((i = 0; i < G1_N; i++)); do s=$((G1_FIRST + i)); hrun "G2C.pn.s$s" G2C pn "$(bin_of pn)" "$s" 1 0 "${av[@]}" || return 1; done
  return 0; }

# ---- the 21 planted and violation-only binaries that do not depend on P come from X1c (PIN TABLE above). X1c built them, ran X1b's isolation checks on them and pinned their sha256; here each is
# copied after its hash is checked, and exec-checked natively once (a cheap guard that the copy runs on the day).
# plb_exec NAME : one native run of the copied binary with a tiny setting; exit 0, 3 or 4 (a plant may fire), the 2.40 line, one stdout line
plb_exec() { launch "$G/$1" -n 2 -r 1 -tick-ms 50 -q 10; facts_run "$CUR.out"
  case $LAST_RC in 0|3|4) ;; *) return 1;; esac
  [ "$F_LIBC_OK" = 1 ] && [ "$F_NOUT" = 1 ]; }
# plb_pin NAME : the pinned sha256 of one of the 21 binaries (unknown-pin for any other name)
plb_pin() { echo "${PLB_PINS[$1]:-unknown-pin}"; }
# pin_table_ok : every pin of the table, the two X1c record pins and the X1c work dir name are filled (64 hex digits, no placeholder)
pin_table_ok() { local n re='^[0-9a-f]{64}$'
  [[ $PIN_X1C_CASES =~ $re ]] && [[ $PIN_X1C_STEPS =~ $re ]] || return 1
  case $X1C in *DATA-DEPENDENT*) return 1;; esac
  [ "${#PLB_NAMES[@]}" = 21 ] || return 1
  for n in "${PLB_NAMES[@]}"; do [[ $(plb_pin "$n") =~ $re ]] || return 1; done
  return 0; }
# x1c_binaries : the X1c preconditions, then for each of the 21 names: the hash in the X1c work dir equals the pin (REUSE-PLB-<name>), the copy succeeds (COPY-PLB-<name>), the copy
# has the pin (PLB-SHA-<name>) and runs (EXEC-PLB-<name>). Any failure is GATE-FAIL <name>: nothing is rebuilt, repaired or retried.
x1c_binaries() { local n pin
  [ -d "$X1C" ] || gate_fail REUSE-X1C-DIR
  [ "$(sha_of "$X1C/cases.tsv")" = "$PIN_X1C_CASES" ] || gate_fail REUSE-X1C-RECORD-CASES
  [ "$(sha_of "$X1C/steps.tsv")" = "$PIN_X1C_STEPS" ] || gate_fail REUSE-X1C-RECORD-STEPS
  for n in "${PLB_NAMES[@]}"; do pin=$(plb_pin "$n")
    [ "$(sha_of "$X1C/pin/$n")" = "$pin" ] || gate_fail "REUSE-PLB-$n"
    cp "$X1C/pin/$n" "$G/$n" || gate_fail "COPY-PLB-$n"
    chmod 755 "$G/$n"
    [ "$(sha_of "$G/$n")" = "$pin" ] || gate_fail "PLB-SHA-$n"
    plb_exec "$n" || gate_fail "EXEC-PLB-$n"
  done
  note "X1C-PINS-VERIFIED 21 binaries copied, hashed against the pin table and exec-checked"; }

# ---- PART P: the three binaries of the P2 tree (condlock-P2, tprobe-P2, condlock-novio-P2) come from the P2 box build, pinned in $BIN/glg-p2.sh (DATA-DEPENDENT: P2); the layout of the P2 work
# dir is the X1c layout: $P2_DIR/pin/<name>, with cases.tsv and steps.tsv beside it. Nothing of the first P tree is read.
p2_pin() { case $1 in condlock-P2) echo "$P2_PIN_BIN";; tprobe-P2) echo "$P2_PIN_TPROBE";; condlock-novio-P2) echo "$P2_PIN_NOVIO";; *) echo unknown-pin;; esac; }
# p2_pin_ok : the P2 work dir name and all five P2 pins are filled (64 lowercase hex digits, no placeholder)
p2_pin_ok() { local re='^[0-9a-f]{64}$' v
  case $P2_DIR in ""|*DATA-DEPENDENT*) return 1;; esac
  for v in "$P2_PIN_CASES" "$P2_PIN_STEPS" "$P2_PIN_BIN" "$P2_PIN_TPROBE" "$P2_PIN_NOVIO"; do [[ $v =~ $re ]] || return 1; done
  return 0; }
# p2_binaries : the P2 work dir and its two records hash to the pins; for each of the three names: the hash in the P2 work dir equals the pin (REUSE-P2-<name>), the copy succeeds
# (COPY-P2-<name>), the copy has the pin (P2-SHA-<name>); the two condlock builds also run natively once (EXEC-P2-<name>). Any failure is GATE-FAIL <name>; nothing is rebuilt or retried.
p2_binaries() { local n pin
  [ -d "$P2_DIR" ] || gate_fail REUSE-P2-DIR
  [ "$(sha_of "$P2_DIR/cases.tsv")" = "$P2_PIN_CASES" ] || gate_fail REUSE-P2-RECORD-CASES
  [ "$(sha_of "$P2_DIR/steps.tsv")" = "$P2_PIN_STEPS" ] || gate_fail REUSE-P2-RECORD-STEPS
  for n in condlock-P2 tprobe-P2 condlock-novio-P2; do pin=$(p2_pin "$n")
    [ "$(sha_of "$P2_DIR/pin/$n")" = "$pin" ] || gate_fail "REUSE-P2-$n"
    cp "$P2_DIR/pin/$n" "$G/$n" || gate_fail "COPY-P2-$n"
    chmod 755 "$G/$n"
    [ "$(sha_of "$G/$n")" = "$pin" ] || gate_fail "P2-SHA-$n"
  done
  for n in condlock-P2 condlock-novio-P2; do plb_exec "$n" || gate_fail "EXEC-P2-$n"; done
  note "P2-PINS-VERIFIED 3 binaries copied, hashed against glg-p2.sh and exec-checked"; }

# ---- gates : infrastructure, reuse and Hermit checks. A gate failure is never a result (GATE-FAIL in steps.tsv, exit 5, nothing measured).
gates() { local a A
  [ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
  case "$RND:$G0S" in 500:319101|250:319511|125:319521|60:319531|30:319541) ;; *) gate_fail ARGS;; esac
  case $PART in A|P) ;; *) gate_fail ARGS;; esac
  note "ARGS rnd=$RND g0s=$G0S part=$PART"; echo "$RND $G0S $PART" > "$O/args.txt"
  if [ "$PART" = P ]; then p2_pin_ok || gate_fail P2-PIN-TABLE; else pin_table_ok || gate_fail PIN-TABLE; fi
  sudo -n /bin/true > /dev/null 2>&1 || gate_fail SUDO
  grep -E ' /dev/shm ' /proc/mounts | grep -q noexec && gate_fail NOEXEC
  [ "$(df -Pm "$R" | awk 'NR==2{print $4}')" -ge 1024 ] || gate_fail FREE-1GB
  for pth in "$R" "$G"; do other_has 5 "$pth" || gate_fail PERMS; done
  for pth in "$SUMD" "$TMPS"; do [ "$(stat -c %a "$pth")" = 1777 ] || gate_fail PERMS-1777; done
  taskset -c $CORES true || gate_fail CORES
  pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && gate_fail BUSY
  [ -x "$HBIN" ] && [ "$(sha_of "$HBIN")" = "$HSHA" ] || gate_fail HERMIT-SHA
  [ -d "$X1B" ] || gate_fail REUSE-DIR
  [ "$(sha_of "$X1B/cases.tsv")" = "$PIN_X1B_CASES" ] || gate_fail REUSE-RECORD-CASES
  [ "$(sha_of "$X1B/steps.tsv")" = "$PIN_X1B_STEPS" ] || gate_fail REUSE-RECORD-STEPS
  [ "$(sha_of "$X1B/ck/examples/glibc-condlock/scripts/classify.sh")" = "$PIN_CLS" ] || gate_fail REUSE-CLASSIFY
  [ "$(sha_of "$CLS")" = "$PIN_CLS" ] || gate_fail CLASSIFY-SHA
  if [ "$PART" = A ]; then
  for a in u c; do A=${a^^}
    [ "$(sha_of "$X1B/ck/b-$A/condlock")" = "$(pin_of BIN $a)" ] || gate_fail "REUSE-BIN-$A"
    [ "$(sha_of "$X1B/$A/sysroot/usr/lib/libc.a")" = "$(pin_of LIBC $a)" ] || gate_fail "REUSE-LIBC-$A"
    cp "$X1B/ck/b-$A/condlock" "$G/condlock-$A" || gate_fail "COPY-$A"
    chmod 755 "$G/condlock-$A"
    [ "$(sha_of "$G/condlock-$A")" = "$(pin_of BIN $a)" ] || gate_fail "BIN-SHA-$A"
  done
  # shellcheck disable=SC2043  # one arm only (C), kept as a loop beside the arm loop above
  for a in c; do A=${a^^}
    [ "$(sha_of "$X1B/ck/b-$A/tprobe")" = "$(pin_of TPROBE $a)" ] || gate_fail "REUSE-TPROBE-$A"
    cp "$X1B/ck/b-$A/tprobe" "$G/tprobe-$A" || gate_fail "COPY-TPROBE-$A"
    chmod 755 "$G/tprobe-$A"
    [ "$(sha_of "$G/tprobe-$A")" = "$(pin_of TPROBE $a)" ] || gate_fail "TPROBE-SHA-$A"
  done
  [ "$(sha_of "$X1B/ck/b-C/sdk/libhairspring.a")" = "$(pin_of SDK c)" ] || gate_fail REUSE-SDK-C
  fi
  python3 -B "$GE" selftest > "$O/selftest.txt" 2>&1 || gate_fail EVAL-SELFTEST
  if [ "$PART" = A ]; then
  for a in u c; do A=${a^^}
    launch "$G/condlock-$A" -n 2 -r 1 -tick-ms 50 -q 10
    facts_run "$CUR.out"
    { [ "$LAST_RC" = 0 ] && [ "$F_LIBC_OK" = 1 ] && [ "$F_NOUT" = 1 ]; } || gate_fail "EXEC-$A"
  done
  fi
  hcore GATE-H "$G0S" /bin/echo smoke
  { [ "$H_RC" = 0 ] && [ "$(cat "$O/GATE-H.stdout")" = smoke ]; } || gate_fail HERMIT-SMOKE
  [ "$(python3 -B "$BIN/fpx.py" "$SUMD/GATE-H.summary.json" | cut -d' ' -f1)" != NOSUMMARY ] || gate_fail HERMIT-SUMMARY
  if [ "$PART" = P ]; then p2_binaries; hcore GATE-C "$SMOKE_SEED" "$G/condlock-P2" -n 2 -r 1 -tick-ms "$TICK" -q "$QUORUM"
  else hcore GATE-C "$SMOKE_SEED" "$G/condlock-C" -n 2 -r 1 -tick-ms "$TICK" -q "$QUORUM"; fi
  facts_run "$O/GATE-C.stdout"
  { [ "$H_RC" = 0 ] && [ "$F_LIBC_OK" = 1 ] && [ "$F_NOUT" = 1 ]; } || gate_fail HERMIT-CONDLOCK
  if [ "$PART" = A ]; then x1c_binaries; fi
  (cd "$G" && sha256sum condlock-* tprobe-*) > "$O/bins.sha"
  note GATES-PASSED; }

# bin_check : every binary of $G, the classifier, Hermit and the driver's own scripts are hashed again at the end
bin_check() { local a A f n
  if [ "$PART" = P ]; then
    for n in condlock-P2 tprobe-P2 condlock-novio-P2; do [ "$(sha_of "$G/$n")" = "$(p2_pin "$n")" ] || note "BIN-CHANGED $n"; done
  else
    for n in "${PLB_NAMES[@]}"; do [ "$(sha_of "$G/$n")" = "$(plb_pin "$n")" ] || note "BIN-CHANGED $n"; done
    for a in u c; do A=${a^^}; [ "$(sha_of "$G/condlock-$A")" = "$(pin_of BIN $a)" ] || note "BIN-CHANGED condlock-$A"; done
    # shellcheck disable=SC2043  # one arm only (C)
    for a in c; do A=${a^^}; [ "$(sha_of "$G/tprobe-$A")" = "$(pin_of TPROBE $a)" ] || note "BIN-CHANGED tprobe-$A"; done
  fi
  (cd "$G" && sha256sum -c "$O/bins.sha" --quiet) > "$O/bins-end.txt" 2>&1 || note "BIN-CHANGED $(grep -m1 FAILED "$O/bins-end.txt" || head -n 1 "$O/bins-end.txt")"
  [ "$(sha_of "$CLS")" = "$PIN_CLS" ] || note "BIN-CHANGED classify.sh"
  [ "$(sha_of "$HBIN")" = "$HSHA" ] || note "BIN-CHANGED hermit"
  for f in glg.sh glg_eval.py guard.sh fpx.py; do sha256sum "$BIN/$f" >> "$O/end-helpers.txt"; done
  if [ "$PART" = P ]; then sha256sum "$P2F" >> "$O/end-helpers.txt"; fi; }

# rootowned : everything under $R that is not owned by ubuntu (Hermit runs as root), listed and LEFT IN PLACE; errors from unreadable root-created directories go to the same file
rootowned() { find "$R" ! -user ubuntu -printf '%u %m %p\n' > "$R/rootowned.txt" 2>&1; note "ROOTOWNED $(grep -c . "$R/rootowned.txt") lines (root-owned entries and permission errors, left in place)"; }

# ---- test seam: the Mac stub tests source this file with GLG_TEST=1 to load the functions without running anything; a real run refuses the variable
if [ -n "${GLG_TEST:-}" ]; then
  if [ "${BASH_SOURCE[0]}" = "$0" ]; then echo "GLG_TEST is set in a real run: refused" >&2; exit 5; fi
  return 0
fi

echo "$SID $T0" > "$R/sid.txt"
mkdir -p "$O" "$RUNS" "$R/cur"
if [ -e "$R/steps.tsv" ] || [ -e "$R/runs.tsv" ]; then echo "GATE-FAIL EXISTS: a previous run left files in $R" >&2; exit 5; fi
: > "$R/steps.tsv"; : > "$R/runs.tsv"; : > "$R/tp.tsv"; : > "$R/dumps.tsv"; snapshot pre
env | sort > "$O/env.txt"
gates
PASSED=0
if [ "$PART" = P ]; then
  if step_tp && gate_step tp && step_g0 && gate_step g0 && pause_ok && step_g1 && gate_step g1 && step_g2c && gate_step g2c; then PASSED=1; fi
else
  if step_tp && gate_step tp && step_g0 && gate_step g0 && pause_ok && step_dposh && gate_step dposh && step_dstallh && gate_step dstallh && step_g2a; then PASSED=1; fi
fi
note "LADDER-DONE passed=$PASSED"
bin_check
rootowned
snapshot post; leftovers > "$R/leftover.txt"; note "LEFTOVER=$(wc -l < "$R/leftover.txt") DONE"
