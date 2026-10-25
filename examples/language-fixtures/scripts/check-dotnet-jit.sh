#!/bin/sh
# Run-time evidence of what the JIT made of the .NET fixture's two worker loops.
#
#   check-dotnet-jit.sh <profiles.manifest> <LangAt.dll> <dotnet>
#
# The window of the lost-update fixture is the three steps in LangAt.Race.RacyIterations (and LockedIterations): a load of the
# plain 64-bit counter field, a call of Spin (marked NoInlining) and a store of the same field. This script does not rely on
# that design. For the default environment and for each arm of profiles.manifest it runs buggy, fixed and wide with
#   DOTNET_JitStdOutFile=<file> DOTNET_JitDisasmSummary=1 DOTNET_JitDisasm="RacyIterations LockedIterations"
# and reads what the JIT printed: one summary line for every compilation (with its tier: Tier0, Instrumented Tier0, an OSR
# compile, Tier1, FullOpts ...) and one assembly listing for each. It fails unless, for the worker method the mode runs
# (RacyIterations for buggy and wide, LockedIterations for fixed):
#   - the method was compiled at least once, and there is one listing for every compilation;
#   - every listing holds a load of a 64-bit field (the base register is not the frame or stack pointer), then exactly one call,
#     then a store to the same offset (the counter): the call, if the listing names its target, is Spin, and no helper call or
#     other call sits in the window;
# and the other worker method was never compiled (it is not run in that mode). Positive control, so an output the check could
# not read is not taken for a pass: LangAt.Race:Spin must be among the compilations. Both x86-64 (Intel syntax) and arm64
# listings are read. The tier of every compilation is printed; the check does not decide which tier is acceptable, only that
# the window is in every one of them.
# Not covered: a method the runtime recompiles after the run ends, and the hardware's own reordering.
# DOTNET names the runtime directory's dotnet through the third argument. Nothing here runs Hermit.
# Exits 0 when every run was clean, 1 when one was not, 64 for a usage error.

set -u

if [ $# -ne 3 ]; then
  echo "usage: $0 <profiles.manifest> <LangAt.dll> <dotnet>" >&2
  exit 64
fi
here=$(cd "$(dirname "$0")" && pwd)
manifest=$1
dll=$2
dotnet=$3
if [ ! -r "$manifest" ]; then
  echo "FAIL $manifest cannot be read" >&2
  exit 1
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/dotnet-jit.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
root=$(dirname "$(readlink -f "$dotnet" 2>/dev/null || echo "$dotnet")")
status=0

# window_check <listing file>: one line per worker-method listing: "<method> <tier> OK <detail>" or "<method> <tier> BAD <reason>".
window_check() {
  awk '
    function finish(   i, j, k, calls, found, why, nloads) {
      if (method == "") return
      found = 0; why = "no load of a 64-bit field is followed by a store to the same offset"
      for (i = 1; i <= n && !found; i++) {
        if (kind[i] != "L") continue
        for (j = i + 1; j <= n && !found; j++) {
          if (kind[j] != "S" || off[j] != off[i]) continue
          calls = 0; bad = ""
          for (k = i + 1; k < j; k++) if (kind[k] == "C") { calls++; cline = ln[k]; target = off[k]; if (target ~ /CORINFO_HELP|[:.]/ && target !~ /Spin/) bad = target }
          if (calls == 1 && bad == "") { found = 1; detail = "load " off[i] " (line " ln[i] "), call (line " cline "), store " off[j] " (line " ln[j] ")" }
          else if (calls == 0) why = "no call between the load and the store of offset " off[i]
          else if (bad != "") why = "a call to " bad " sits between the load and the store of offset " off[i]
          else why = calls " calls between the load and the store of offset " off[i]
          break
        }
      }
      if (found) printf "%s %s OK %s\n", method, tier, detail
      else printf "%s %s BAD %s\n", method, tier, why
    }
    /^; Assembly listing for method / {
      finish()
      n = 0; method = $0; sub(/^; Assembly listing for method /, "", method)
      tier = method; sub(/^.*\(/, "", tier); sub(/\)[ \t]*$/, "", tier); gsub(/ /, "_", tier)
      sub(/\(.*$/, "", method)
      if (method !~ /^LangAt\.Race:(Racy|Locked)Iterations$/) method = ""
      next
    }
    method == "" { next }
    {
      line = $0
      # arm64: ldr x0, [x1, #0x30]  /  str x0, [x2, #0x30]  /  bl, blr
      if (match(line, /^[ \t]+(ldr|ldur)[ \t]+x[0-9]+, \[[a-z0-9]+, #0x[0-9A-Fa-f]+\]/)) {
        s = substr(line, RSTART, RLENGTH); base = s; sub(/^.*\[/, "", base); sub(/,.*$/, "", base)
        o = s; sub(/^.*#/, "", o); sub(/\]$/, "", o)
        if (base != "fp" && base != "sp" && base != "x29") { n++; kind[n] = "L"; off[n] = tolower(o); ln[n] = NR; lastload = n }
        next
      }
      if (match(line, /^[ \t]+(str|stur)[ \t]+x[0-9]+, \[[a-z0-9]+, #0x[0-9A-Fa-f]+\]/)) {
        s = substr(line, RSTART, RLENGTH); base = s; sub(/^.*\[/, "", base); sub(/,.*$/, "", base)
        o = s; sub(/^.*#/, "", o); sub(/\]$/, "", o)
        if (base != "fp" && base != "sp" && base != "x29") { n++; kind[n] = "S"; off[n] = tolower(o); ln[n] = NR }
        next
      }
      if (match(line, /^[ \t]+(bl|blr)[ \t]+[^ \t].*$/)) {
        s = line; sub(/^[ \t]+(bl|blr)[ \t]+/, "", s); sub(/[ \t;].*$/, "", s); n++; kind[n] = "C"; off[n] = s; ln[n] = NR; next
      }
      # x86-64 (Intel syntax): mov rax, qword ptr [rbx+0x30]  /  mov qword ptr [rbx+0x30], rax  /  call [...]
      if (match(line, /^[ \t]+mov[ \t]+[a-z0-9]+, qword ptr \[[a-z0-9]+\+0x[0-9A-Fa-f]+\]/)) {
        s = substr(line, RSTART, RLENGTH); base = s; sub(/^.*\[/, "", base); sub(/\+.*$/, "", base)
        o = s; sub(/^.*\+/, "", o); sub(/\]$/, "", o)
        if (base != "rbp" && base != "rsp") { n++; kind[n] = "L"; off[n] = tolower(o); ln[n] = NR }
        next
      }
      if (match(line, /^[ \t]+mov[ \t]+qword ptr \[[a-z0-9]+\+0x[0-9A-Fa-f]+\], [a-z0-9]+/)) {
        s = substr(line, RSTART, RLENGTH); base = s; sub(/^.*\[/, "", base); sub(/\+.*$/, "", base)
        o = s; sub(/^.*\+/, "", o); sub(/\].*$/, "", o)
        if (base != "rbp" && base != "rsp") { n++; kind[n] = "S"; off[n] = tolower(o); ln[n] = NR }
        next
      }
      if (match(line, /^[ \t]+call[ \t]+[^ \t].*$/)) {
        s = line; sub(/^[ \t]+call[ \t]+/, "", s); n++; kind[n] = "C"; off[n] = s; ln[n] = NR; next
      }
    }
    END { finish() }
  ' "$1"
}

# summary_lines <jit file> <method>: the compile summary lines of one method, with the tier.
summary_lines() {
  grep -E "JIT compiled LangAt\.Race:$2\(" "$1" | sed -E 's/^ *[0-9]+: JIT compiled ([^ ]+) \[([^],]*).*$/\1 \2/'
}

# check_run <arm> <mode> <env words...>: one run of the fixture and the reading of what the JIT printed.
check_run() {
  arm=$1
  mode=$2
  shift 2
  rm -f "$tmp/jit.txt"
  # The words are separate arguments to env on purpose.
  env "$@" DOTNET_JitStdOutFile="$tmp/jit.txt" DOTNET_JitDisasmSummary=1 DOTNET_JitDisasm="RacyIterations LockedIterations" \
    "$dotnet" "$dll" "$mode" >"$tmp/out" 2>"$tmp/err" </dev/null
  rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "FAIL arm $arm, $mode: exit status $rc"
    cat "$tmp/err"
    return 1
  fi
  if [ ! -s "$tmp/jit.txt" ]; then
    echo "FAIL arm $arm, $mode: the JIT printed nothing, so this run proves nothing"
    return 1
  fi
  if ! grep -Eq 'JIT compiled LangAt\.Race:Spin\(' "$tmp/jit.txt"; then
    echo "FAIL arm $arm, $mode: LangAt.Race:Spin was not among the compilations, so this is not the program the check reads"
    return 1
  fi
  want=RacyIterations
  other=LockedIterations
  if [ "$mode" = fixed ]; then
    want=LockedIterations
    other=RacyIterations
  fi
  if [ -n "$(summary_lines "$tmp/jit.txt" "$other")" ]; then
    echo "FAIL arm $arm, $mode: $other was compiled, but this mode does not run it"
    return 1
  fi
  compiles=$(summary_lines "$tmp/jit.txt" "$want")
  if [ -z "$compiles" ]; then
    echo "FAIL arm $arm, $mode: $want was never compiled"
    return 1
  fi
  window_check "$tmp/jit.txt" >"$tmp/windows.txt"
  listed=$(grep -c "^LangAt\.Race:$want " "$tmp/windows.txt")
  compiled=$(printf '%s\n' "$compiles" | wc -l | tr -d ' ')
  if [ "$listed" -ne "$compiled" ]; then
    echo "FAIL arm $arm, $mode: $compiled compilations of $want but $listed listings"
    return 1
  fi
  bad=$(grep -c " BAD " "$tmp/windows.txt")
  while read -r method tier verdict detail; do
    echo "     $method [$(printf '%s' "$tier" | tr '_' ' ')]: $verdict ${detail:-}"
  done <"$tmp/windows.txt"
  if [ "$bad" -ne 0 ]; then
    echo "FAIL arm $arm, $mode: a compilation of $want does not show a load, one call and a store of the same field"
    return 1
  fi
  echo "ok   arm $arm, $mode: $compiled compilation(s) of $want, each with the load, the call of Spin and the store"
}

# check_arm <arm> <env words...>: the three modes.
check_arm() {
  arm=$1
  shift
  echo "arm $arm: ${*:-(nothing added)}"
  for mode in buggy fixed wide; do
    check_run "$arm" "$mode" "$@" || return 1
  done
}

check_arm default DOTNET_ROOT="$root" || status=1
for arm in a b; do
  if ! words=$(sh "$here/../dotnet/profile-env.sh" "$arm" "$root" "$manifest"); then
    echo "FAIL arm $arm: the manifest $manifest is bad"
    status=1
    continue
  fi
  # shellcheck disable=SC2086  # the words are NAME=value, one per line, none with a space
  check_arm "$arm" $words || status=1
done
exit $status
