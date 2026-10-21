#!/bin/sh
# Tests of check-native.sh, run-checks.sh and check-sources.sh with a fake fixture program, so they need no compiler.
#
# The fake is one program with the contract's argv modes (buggy, fixed, wide). FAKE selects how it behaves:
#   (unset)       a perfect program: always exact, so its racy modes never fire
#   racy          buggy and wide lose an update on every other run; fixed never does
#   <defect>      racy, plus one violation of the contract (the cases below name them); each must be rejected

set -u
here=$(dirname "$0")
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0

fake="$tmp/fake"
cat >"$fake" <<'EOF'
#!/bin/sh
usage() {
  echo "usage: fake buggy|fixed|wide" >&2
  exit "${FAKE_USAGE_STATUS:-64}"
}
if [ "${FAKE:-}" = nousage ]; then
  [ $# -eq 1 ] || { echo "ignored" >&2; exit 0; }
else
  [ $# -eq 1 ] || usage
  case $1 in buggy | fixed | wide) ;; *) usage ;; esac
fi
mode=$1
lang=${FAKE_LANG:-t}
state=${FAKE_STATE:?}
n=$(cat "$state/n" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" >"$state/n"

final=4000
case "${FAKE:-}:$mode" in
  :*) ;;
  *:fixed) [ "$FAKE" = fixedfires ] && final=3990 ;;
  *) [ $((n % 2)) -eq 1 ] && final=3990 ;;
esac
falses=0
[ "$final" -ne 4000 ] && falses=1
[ "${FAKE:-}" = disagree ] && falses=0

shown=$mode
[ "${FAKE:-}" = wrongmode ] && shown=buggy
digits=20
[ "${FAKE:-}" = shortfinal ] && digits=10
head="LANGAT v1"
[ "${FAKE:-}" = oldline ] && head="lang=$lang"
id=$lang
[ "${FAKE:-}" = wrongid ] && id=cpp
vals="[$final,4000,4,1000]"
[ "${FAKE:-}" = twovals ] && vals="[$final,4000,0,0]"
hits=1
[ "${FAKE:-}" = twice ] && hits=2

event() { # event <id> <falses> <vals> <hits>
  trues=$((1 - $2))
  echo "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"$1\",\"hits\":   $4,\"trues\":   $trues,\"falses\":   $2,\"first_false\":null,\"first_true\":{\"thread\":\"t#1\",\"ord\":1,\"vals\":$3}}" >&2
}
event "$id.all_started" 0 "[4,0,0,0]" "$hits"
event "$id.counter_exact" "$falses" "$vals" "$hits"
[ "${FAKE:-}" = incomplete ] && echo "hairspring: incomplete catalog: 1 evaluation(s) ran at call sites that are missing from the catalog section" >&2

printf "%s lang=%s mode=%s final=%0${digits}d expect=%020d order=0123456789abcdef rand=0123456789abcdef\n" "$head" "$lang" "$shown" "$final" 4000
[ "${FAKE:-}" = extraline ] && echo "done"
[ "${FAKE:-}" = badexit ] && exit 1
exit 0
EOF
chmod +x "$fake"

expect() { # expect <name> <wanted exit> <command...>
  name=$1
  wanted=$2
  shift 2
  "$@" >/dev/null 2>&1
  got=$?
  if [ "$got" -eq "$wanted" ]; then
    echo "ok   $name"
  else
    echo "FAIL $name: exit $got, want $wanted"
    failures=$((failures + 1))
  fi
}

newstate() { mktemp -d "$tmp/state.XXXXXX"; }

# check <name> <wanted exit> <FAKE value> [extra env...]: check-native.sh on four CPUs with six runs per racy mode.
check() {
  name=$1
  wanted=$2
  behaviour=$3
  shift 3
  expect "$name" "$wanted" env FAKE="$behaviour" FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 "$@" \
    sh "$here/check-native.sh" t "$fake" 6
}

check "a conforming racy program passes" 0 racy
check "a program that never fires fails on several CPUs" 1 ""
expect "a program that never fires is skipped on one CPU" 77 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=1 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
expect "the skip is a failure when parallelism is required" 1 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=1 LANG_AT_REQUIRE_PARALLEL=1 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
check "a wrong lang id is a contract failure, not a skip" 1 racy FAKE_LANG=other
check "the old stdout line without LANGAT v1 is rejected" 1 oldline
check "a line that names the wrong mode is rejected" 1 wrongmode
check "a 10-digit final is rejected" 1 shortfinal
check "an extra stdout line is rejected" 1 extraline
check "a non-zero exit status is rejected" 1 badexit
check "a fixed mode that loses an update is rejected" 1 fixedfires
check "a detector that disagrees with the program is rejected" 1 disagree
check "an id with the wrong prefix is rejected" 1 wrongid
check "witness values other than [final, 4000, 4, 1000] are rejected" 1 twovals
check "a property evaluated twice is rejected" 1 twice
check "a usage error that exits 2 instead of 64 is rejected" 1 racy FAKE_USAGE_STATUS=2
check "a program that ignores a bad argument is rejected" 1 nousage
check "the SDK's incomplete-catalog line is rejected" 1 incomplete

# run-checks.sh: the real artifact, then the negative control (the fixed mode run as buggy must be rejected).
expect "run-checks passes a conforming racy program with its control" 0 env FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks treats the skip as success on one CPU" 0 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=1 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks fails a program that never fires on several CPUs" 1 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks without an artifact is a usage error" 64 sh "$here/run-checks.sh" t

# A stand-in for check-native.sh that passes the first time and reports "skipped" the second time (the negative control).
stub="$tmp/stub-check"
cat >"$stub" <<EOF
#!/bin/sh
n=\$(cat "$tmp/calls" 2>/dev/null || echo 0)
echo \$((n + 1)) >"$tmp/calls"
[ "\$n" -eq 0 ] && exit 0
exit 77
EOF
rm -f "$tmp/calls"
out=$(CHECK_NATIVE="$stub" sh "$here/run-checks.sh" t a 1 2>&1)
rc=$?
if [ "$rc" -eq 0 ] && echo "$out" | grep -q "skip" && ! echo "$out" | grep -q "rejects"; then
  echo "ok   a skipped negative control is reported as skipped, not as a rejection"
else
  echo "FAIL a skipped negative control: exit $rc, output: $out"
  failures=$((failures + 1))
fi

# check-sources.sh: the fixture must not read the seed, /proc, the CPU count or the clock.
clean="$tmp/clean.c"
printf 'int main(void) { return 0; }\n' >"$clean"
expect "check-sources accepts a clean source" 0 sh "$here/check-sources.sh" "$clean"
for token in HAIRSPRING_SEED '/proc/self' sysconf nproc NumCPU hardware_concurrency clock_gettime gettimeofday 'time.Now'; do
  bad="$tmp/bad.c"
  printf 'void f(void) { use("%s"); }\n' "$token" >"$bad"
  expect "check-sources rejects $token" 1 sh "$here/check-sources.sh" "$bad"
done
expect "check-sources without a file is a usage error" 64 sh "$here/check-sources.sh"

# check-disasm.sh: the build record's B2, on canned objdump text of a loop and of a worker's window.
loop_ok="$tmp/loop_ok.s"
cat >"$loop_ok" <<'EOF'
0000000000001230 <lang_at_spin>:
    1230:	test   %esi,%esi
    1232:	je     1250 <lang_at_spin+0x20>
    1240:	imul   %rcx,%rdi
    1244:	add    %rdx,%rdi
    1248:	sub    $0x1,%esi
    124b:	jne    1240 <lang_at_spin+0x10>
    1250:	mov    %rdi,%rax
    1253:	ret
EOF
loop_gone="$tmp/loop_gone.s"
cat >"$loop_gone" <<'EOF'
0000000000001230 <lang_at_spin>:
    1230:	mov    %rdi,%rax
    1233:	test   %esi,%esi
    1235:	je     123a <lang_at_spin+0xa>
    1237:	add    $0x1,%rax
    123a:	ret
EOF
window_ok="$tmp/window_ok.s"
cat >"$window_ok" <<'EOF'
0000000000001300 <worker>:
    1300:	mov    0x2f00(%rip),%r12
    1307:	mov    %r12,%rdi
    130a:	mov    $0x3e8,%esi
    130f:	call   1230 <lang_at_spin>
    1314:	lea    0x1(%r12),%rdx
    1319:	mov    %rdx,0x2ef0(%rip)
    1320:	ret
EOF
window_locked="$tmp/window_locked.s"
cat >"$window_locked" <<'EOF'
0000000000001300 <worker>:
    1300:	mov    $0x1,%eax
    1305:	call   1230 <lang_at_spin>
    130a:	lock xadd %rax,0x2ef0(%rip)
    1310:	ret
EOF
no_call="$tmp/no_call.s"
printf '0000000000001300 <worker>:\n    1300:\tret\n' >"$no_call"
expect "check-disasm finds the loop that survived" 0 sh "$here/check-disasm.sh" loop "$loop_ok"
expect "check-disasm reports a loop the optimiser removed" 1 sh "$here/check-disasm.sh" loop "$loop_gone"
expect "check-disasm accepts a window with a plain load and store" 0 sh "$here/check-disasm.sh" window "$window_ok"
expect "check-disasm rejects a locked read-modify-write in the window" 1 sh "$here/check-disasm.sh" window "$window_locked"
expect "check-disasm reports a worker with no call to the busy loop" 2 sh "$here/check-disasm.sh" window "$no_call"
expect "check-disasm with an unknown check is a usage error" 64 sh "$here/check-disasm.sh" other "$loop_ok"

exit $failures
