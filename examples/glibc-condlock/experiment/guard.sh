#!/usr/bin/env bash
# guard.sh : sourced by the N and G drivers (byte-identical in both specs). The conventions of F0-e's fe.sh, in one place:
#   * the driver's whole ancestor chain is computed once and excluded from the work-dir scan (rev 3 of F0-e);
#   * ok_to_start refuses on STOP / HALT / size cap / time budget, WAITS (counted in the time cap) while a foreign job runs, and refuses while any process of
#     any user, in any session, has "$R/" in its arguments (the leftover.sh filter);
#   * a stop signal (TERM, INT, HUP) writes HALT, a post snapshot and the leftover scan, then exits 6 (the BL1 / G0m0 'finish' convention);
#   * gate_fail NAME writes GATE-FAIL, the post snapshot and the leftover scan, then exits 5.
# The caller sets R (work dir), T0 (epoch seconds at start) and NEWBY (epoch seconds after which no new unit of work starts) before sourcing it.
# shellcheck shell=bash
R=${R:?R is not set}
T0=${T0:?T0 is not set}
NEWBY=${NEWBY:?NEWBY is not set}
IDLE_BUDGET=${IDLE_BUDGET:-900}     # seconds the whole job may spend waiting for a foreign job to leave; these seconds are inside the time cap
IDLE_USED=0
SIZE_CAP_MB=${SIZE_CAP_MB:-400}
SID=$(ps -o sid= -p $$ | tr -d ' ')
# the driver's whole ancestor chain (this shell up to pid 1), once, as " pid pid ... ": the launch shell, nohup/timeout and anything above may have $R/ in their arguments
ANC=" "; p=$$
while [ -n "$p" ] && [ "$p" -gt 1 ]; do ANC="$ANC$p "; p=$(ps -o ppid= -p "$p" | tr -d ' '); done
unset p

note() { printf '%s\t%s\n' "$(date +%s)" "$*" >> "$R/steps.tsv"; }
# every process of ANY user, in ANY session, whose arguments contain "$R/" (the leftover.sh filter). Excluded by pid: every pid of the driver's ancestor chain ANC
# and the driver's own direct children (the pipeline's ps and awk exist as forks of the driver for a moment).
procs_with_rdir() { ps -eo pid=,ppid=,user=,sid=,args= | RP="$R/" awk -v me="$$" -v anc="$ANC" 'index(anc, " " $1 " ") == 0 && $2!=me && index($0,ENVIRON["RP"])'; }
snapshot() { { date -u +%FT%TZ; cat /proc/loadavg; ps -eo pid,ppid,pgid,sid,ni,psr,pcpu,etimes,stat,comm --sort=-pcpu | head -30
    pgrep -a -x 'hermit|qemu-system-x86|cargo|rustc|make|cc1|cc1plus|gdb|hairspring'; } > "$R/snap-$1.txt" 2>&1
  ls -1 /tmp | sort > "$R/tmp-$1.txt" 2>&1; ls -1 /dev/shm | sort > "$R/shm-$1.txt" 2>&1; ls -la --time-style=full-iso /var/crash > "$R/crash-$1.txt" 2>&1
  ls -lan --time-style=full-iso /tmp /dev/shm > "$R/own-$1.txt" 2>&1; }
leftovers() { local rp="$R/"; export RP="$rp"; ps -eo pid=,user=,sid=,args= | awk '$2!="ubuntu" && index($0,ENVIRON["RP"])'; pgrep -s "$SID" -a; }
gate_fail() { note "GATE-FAIL $1"; snapshot post; leftovers > "$R/leftover.txt"; exit 5; }
on_signal() { trap '' TERM INT HUP; echo SIGNAL > "$R/HALT"; note "SIGNAL: the driver was told to stop; HALT written"
  snapshot post; leftovers > "$R/leftover.txt"; note "LEFTOVER=$(wc -l < "$R/leftover.txt") SIGNAL-DONE"; exit 6; }
trap on_signal TERM INT HUP
# other_has MASK PATH : the "others" permission bits of PATH include every bit of the octal MASK (5 = read and execute)
other_has() { local m; m=$(stat -c %a "$2") || return 1; [ $(( 8#$m & 8#$1 )) -eq $(( 8#$1 )) ]; }
# foreign_busy : prints the reason and returns 0 when something heavy or foreign runs. Between units of work no hermit of ours exists, so any hermit is foreign.
foreign_busy() {
  if pgrep -x 'qemu-system-x86|cargo|rustc' > /dev/null; then echo "heavy job (qemu, cargo or rustc)"; return 0; fi
  if pgrep -x hermit > /dev/null; then echo "a hermit process exists between units of work"; return 0; fi
  return 1; }
ok_to_start() { local why waited=0
  [ -e "$R/STOP" ] && { note STOPPED; return 1; }
  [ -e "$R/HALT" ] && { note "HALTED: $(cat "$R/HALT") - no further step"; return 1; }
  [ "$(du -sm "$R" | cut -f1)" -ge "$SIZE_CAP_MB" ] && { note "SIZE-CAP: $R is $SIZE_CAP_MB MB or more"; return 1; }
  [ "$(date +%s)" -ge "$NEWBY" ] && { note "BUDGET: past the no-new-work line"; return 1; }
  while why=$(foreign_busy); do
    if [ "$IDLE_USED" -ge "$IDLE_BUDGET" ]; then note "IDLE-EXHAUSTED: $why; waited ${IDLE_USED}s of ${IDLE_BUDGET}s in total"; return 1; fi
    [ "$waited" = 0 ] && note "IDLE-WAIT: $why (inside the time cap; budget ${IDLE_BUDGET}s, used ${IDLE_USED}s)"
    sleep 5; IDLE_USED=$((IDLE_USED + 5)); waited=$((waited + 5))
  done
  [ "$waited" -gt 0 ] && note "IDLE-WAITED ${waited}s (total ${IDLE_USED}s)"
  for _ in 1 2 3 4 5; do procs_with_rdir > "$R/procs-now.txt"; [ -s "$R/procs-now.txt" ] || break; sleep 1; done
  [ -s "$R/procs-now.txt" ] && { note "REFUSED: a process has $R/ in its arguments: $(head -n 1 "$R/procs-now.txt" | cut -c1-200)"; return 1; }
  return 0; }
