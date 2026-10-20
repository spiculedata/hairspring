#!/usr/bin/env bash
# C2E driver: the engine part of C2 (GOC1 rev 1). For every A-u hit seed of the counted windows (up to the cap of c1_eval.py `hits`): one `hairspring test` of the A-u shim target at
# that seed (the engine re-runs a failing seed 3 times and writes a bundle) and, when exactly one bundle was written, one `hairspring replay` of it. The engine is used here only;
# it never counts a hit (the counted runs are c1.sh's direct Hermit runs). The engine's own Hermit environment differs from the direct driver's (GOG 5.6), so whether the engine fails
# at the same seed is an observation, read by c1_eval.py `verdict` (the failing sched-seed must be the hit's own seed), never assumed.
# usage: setsid nohup timeout -k 30 7200 bash $R/bin/c2e.sh $R <QW> <WINNO> > $R/run.out 2>&1 < /dev/null &
# The driver is uid 1000. sudo is used in exactly four places: (1) the gate `sudo -n /bin/true`; (2) the ONE `sudo -n install -d -m 755` that creates the root-owned directories $R/e,
# $R/e/db, $R/e/bundles, $R/e/cwd; (3) the root wrapper of every engine call in ee() (timeout INSIDE sudo; stdout and stderr redirects stay outside, so the logs are ubuntu-owned);
# (4) the clean-up `sudo -n kill` of pids that leftover.sh listed (by hand, not here). Root-owned files will remain in $R/e.
# The C_* variables below are for the Mac test harness only: gate OVERRIDES refuses to run if any C_* variable is set unless C_TEST=1.
set -u
R=$1; QW=$2; WINNO=$3
T0=$(date +%s)
# NEWBY is read by guard.sh (ok_to_start)
# shellcheck disable=SC2034
NEWBY=$((T0 + ${C_HARD:-6600}))            # no new engine call after 110 min
# EDIT: the path of your Hermit binary (the run used the build with the sha256 below; see SCRIPTS/README.md)
HBIN=${C_HBIN:-/path/to/hermit}
HSHA=${C_HSHA:-666a8713d510bcd57cf2ea4c419c23368f8e7f5fb6c279a7b4766a142e62674a}
HSSHA=${C_HSSHA:-06a6e53a0e18f9d8da1a190c31aac860549e3c6220122922fdd04f63c10bbc62}
HS=$R/bin/hairspring; CE=$R/bin/c1_eval.py; GE=$R/bin/g_eval.py
G=$R/g; O=$R/out; PREV=$R/prev
CORES=1,2,3; GP=${C_GP:-/usr/bin:/bin}
CELLS=()
# shellcheck source=guard.sh
. "$R/bin/guard.sh"
echo "$SID $T0" > "$R/sid.txt"
mkdir -p "$O"

# target NAME COMMAND-AS-TOML-ARRAY (the GOG 5.6 target, flag set C, one target)
target() {
  printf '[target.%s]\nkind = "process"\ncommand = %s\ncwd = "%s/e/cwd"\ntrial_timeout_secs = 60\nchaos = true\nmax_timeslice_ns = 100000\nskid_margin = 25\nparallelism = 3\n' "$1" "$2" "$R"
  printf 'hermit_args = ["--base-env=minimal", "--env=GOMAXPROCS=2", "--env=GOTRACEBACK=all"]\n\n[target.%s.budget]\nmax_trials = 10\nwall_secs = 300\n\n' "$1"; }
# ee ID CAP ARGS... : one engine call, as root, cwd = the root-created guest cwd. A cap that fires halts the job (the engine has no signal handler; Hermit may be left).
ee() { local id=$1 cap=$2 rc; shift 2
  ok_to_start || { printf '%s\tNOT-RUN\n' "$id" >> "$R/steps.tsv"; return 1; }
  ( ulimit -c 0; ulimit -f 16384; cd "$R/e/cwd" || exit 99
    sudo -n timeout -k 10 "$cap" env -i PATH=$GP HAIRSPRING_HERMIT=$HBIN nice -n 10 taskset -c $CORES \
      "$HS" --config "$R/cfg/ge.toml" --db "$R/e/db/h.db" "$@" ) < /dev/null > "$O/$id.out" 2> "$O/$id.err"
  rc=$?; echo $rc > "$O/$id.rc"; note "$id rc=$rc"
  if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then echo "$id rc=$rc" > "$R/HALT"; note "TIMEOUT-HALT $id rc=$rc: no further step; root processes may be left (section 11)"; fi
  return 0; }

# step_engine : the engine test and replay of every listed A-u hit seed
step_engine() { local s nb b
  mapfile -t CELLS < <(python3 -B "$CE" hits "$PREV" 2>> "$O/gate-errors.txt")
  case ${CELLS[0]:-NO-VERDICT} in
    NONE) note "C2E-COMPLETE no A-u hit in the counted windows; nothing to confirm"; return 0;;
    "HITS "*) note "C2E ${CELLS[0]}";;
    *) note "HALT-RULE C1-HALT/EVAL-ERROR ${CELLS[0]:-NO-VERDICT}"; return 2;;
  esac
  for s in "${CELLS[@]:1}"; do
    ee "C2E.$s.t" 300 test scau --seed "$s" --bundle-dir "$R/e/bundles/$s" --format json || return 1
    nb=$(find "$R/e/bundles/$s" -maxdepth 1 -type f -name '*.hairspring' 2> /dev/null | wc -l | tr -d ' '); note "BUNDLES $s $nb"
    if [ "$nb" = 1 ]; then
      b=$(find "$R/e/bundles/$s" -maxdepth 1 -type f -name '*.hairspring')
      ee "C2E.$s.r" 120 replay "$b" || return 1
    else note "C2E.$s.r NOT-RUN: $nb bundle files"; fi
  done
  note "C2E-COMPLETE every listed hit seed has its engine test and, where exactly one bundle was written, its replay"
  return 0; }

if [ -e "$R/steps.tsv" ] || [ -e "$R/runs.tsv" ] || [ -e "$R/e" ]; then echo "GATE-FAIL EXISTS: a previous run left files in $R" >&2; exit 5; fi
: > "$R/steps.tsv"; : > "$R/runs.tsv"; snapshot pre
env | sort > "$O/env.txt"
# GATES (infrastructure; a gate failure is never a result)
[ "${BASH_VERSINFO[0]}" -ge 5 ] || gate_fail BASH5
if env | grep -q '^C_' && [ "${C_TEST:-0}" != 1 ]; then env | grep '^C_' > "$O/overrides.txt"; gate_fail OVERRIDES; fi
case $QW in 500|250) ;; *) gate_fail ARGS;; esac
{ [[ $WINNO =~ ^[0-9]+$ ]] && [ "$WINNO" -ge 1 ] && [ "$WINNO" -le 20 ]; } || gate_fail ARGS
note "ARGS mode=engine qw=$QW win=$WINNO"; echo "engine $QW $WINNO" > "$O/args.txt"
sudo -n /bin/true > /dev/null 2>&1 || gate_fail SUDO
grep -E ' /dev/shm ' /proc/mounts | grep -q noexec && gate_fail NOEXEC
[ "$(df -Pm "$R" | awk 'NR==2{print $4}')" -ge 1024 ] || gate_fail FREE-1GB
for pth in "$R" "$R/g" "$HS" "$G/u-shim"; do other_has 5 "$pth" || gate_fail PERMS; done
[ "$(stat -c %a "$R/cfg")" = 1777 ] || gate_fail PERMS-1777
pgrep -x 'hermit|qemu-system-x86|cargo|rustc' > /dev/null && gate_fail BUSY
[ "$(sha256sum "$HBIN" | cut -d' ' -f1)" = "$HSHA" ] || gate_fail HERMIT-SHA
[ "$(sha256sum "$G/u-shim" | cut -d' ' -f1)" = "${C_PIN_U_SHIM:-282fddca507d718fa255c3fa35c3ab6ec9d98c217f839deb80d8e48c1d6da822}" ] || gate_fail BIN-SHA-u-shim
[ "$(sha256sum "$HS" | cut -d' ' -f1)" = "$HSSHA" ] || gate_fail HS-SHA
"$HS" --version > "$O/hs-version.txt" 2>&1 || gate_fail HS-EXEC
python3 -B "$GE" selftest > "$O/selftest-g.txt" 2>&1 || gate_fail EVAL-SELFTEST
python3 -B "$CE" selftest > "$O/selftest-c1.txt" 2>&1 || gate_fail EVAL-SELFTEST
v=$(python3 -B "$CE" prevcheck "$PREV" "$QW" 2>> "$O/gate-errors.txt"); note "PREV-CHECK ${v:-NO-VERDICT}"
case $v in OK*) ;; *) gate_fail PREV-BAD;; esac
sudo -n install -d -m 755 "$R/e" "$R/e/db" "$R/e/bundles" "$R/e/cwd" || gate_fail INSTALL
target scau "[\"$G/u-shim\", \"-mode=std\", \"-q\", \"$QW\"]" > "$R/cfg/ge.toml"
sha256sum "$R/cfg/ge.toml" > "$O/ge.toml.sha"
"$HS" --config "$R/cfg/ge.toml" --db "$R/cfg/check.db" targets > "$O/targets.txt" 2>&1 || gate_fail CONFIG
grep -q "^scau " "$O/targets.txt" || gate_fail CONFIG
note "GATES-PASSED"

# THE WINDOW
step_engine; RC=$?
note "WINDOW-RC $RC"
[ "$(sha256sum "$G/u-shim" | cut -d' ' -f1)" = "${C_PIN_U_SHIM:-282fddca507d718fa255c3fa35c3ab6ec9d98c217f839deb80d8e48c1d6da822}" ] || note "BIN-CHANGED u-shim"
[ "$(sha256sum "$HS" | cut -d' ' -f1)" = "$HSSHA" ] || note "BIN-CHANGED hairspring"
snapshot post; leftovers > "$R/leftover.txt"
note "WINDOW-WALL $(( $(date +%s) - T0 ))"; note "LEFTOVER=$(wc -l < "$R/leftover.txt") DONE"
