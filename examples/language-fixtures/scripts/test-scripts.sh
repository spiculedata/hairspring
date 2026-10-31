#!/bin/sh
# Tests of check-native.sh, run-checks.sh and check-sources.sh with a fake fixture program, so they need no compiler.
#
# The fake is one program with the contract's argv modes (buggy, fixed, wide). FAKE selects how it behaves:
#   (unset)       a perfect program: always exact, so its racy modes never fire
#   racy          buggy and wide lose an update on every other run; fixed never does
#   <defect>      racy, plus one violation of the contract (the cases below name them); each must be rejected

set -u
# CI sets this for the real checks; these tests set it themselves where they need it.
unset LANG_AT_REQUIRE_PARALLEL
here=$(dirname "$0")
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0

fake="$tmp/fake"
cat >"$fake" <<'EOF'
#!/bin/sh
usage() {
  # The SDKs report to stderr at exit even after a usage error, so protocol lines may precede or follow the usage line.
  [ "${FAKE:-}" = usageevents ] && echo 'HAIRSPRING_EVENT {"v":1,"kind":"hello"}' >&2
  if [ "${FAKE:-}" = nousagetext ]; then
    echo 'HAIRSPRING_EVENT {"v":1,"kind":"end"}' >&2
  else
    echo "usage: fake buggy|fixed|wide" >&2
  fi
  [ "${FAKE:-}" = usageout ] && echo "usage on stdout too"
  exit "${FAKE_USAGE_STATUS:-64}"
}
if [ "${FAKE:-}" = nousage ]; then
  [ $# -eq 1 ] || { echo "ignored" >&2; exit 0; }
elif [ "${FAKE:-}" = extraok ]; then
  # accepts any number of arguments after a valid mode
  case ${1:-} in buggy | fixed | wide) ;; *) usage ;; esac
elif [ "${FAKE:-}" = switchok ]; then
  # like the Python fixture: an optional second argument of exactly --switch=1.0
  [ $# -eq 1 ] || { [ $# -eq 2 ] && [ "$2" = "--switch=1.0" ]; } || usage
  case $1 in buggy | fixed | wide) ;; *) usage ;; esac
elif [ "${FAKE:-}" = switchany ]; then
  [ $# -le 2 ] || usage
  case ${1:-} in buggy | fixed | wide) ;; *) usage ;; esac
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

# FAKE_K is the K the fake pretends to run at (default 1000): the full count is 4 x K.
k=${FAKE_K:-1000}
full=$((4 * k))
lost=10
[ "$full" -le 10 ] && lost=1
final=$full
case "${FAKE:-}:$mode" in
  :*) ;;
  *:fixed) [ "$FAKE" = fixedfires ] && final=$((full - lost)) ;;
  *) [ $((n % 2)) -eq 1 ] && final=$((full - lost)) ;;
esac
# FAKE_UP=1: a firing run applies updates twice instead of losing them (final above the full count)
[ "${FAKE_UP:-}" = 1 ] && [ "$final" -ne "$full" ] && final=$((full + lost))
falses=0
[ "$final" -ne "$full" ] && falses=1
[ "${FAKE:-}" = disagree ] && falses=0

shown=$mode
[ "${FAKE:-}" = wrongmode ] && shown=buggy
digits=20
[ "${FAKE:-}" = shortfinal ] && digits=10
head="LANGAT v1"
[ "${FAKE:-}" = oldline ] && head="lang=$lang"
id=$lang
[ "${FAKE:-}" = wrongid ] && id=cpp
vals="[$final,$full,4,$k]"
[ "${FAKE:-}" = twovals ] && vals="[$final,$full,0,0]"
[ "${FAKE:-}" = garbage ] && vals="[7,1,2,3]"
thread="t#5"
[ "$lang" = go ] && thread="go"
[ "${FAKE:-}" = workerthread ] && thread="worker-1"
[ "${FAKE:-}" = workerkey ] && thread="t#2"
hits=1
[ "${FAKE:-}" = twice ] && hits=2

event() { # event <id> <falses> <vals> <hits>
  trues=$((1 - $2))
  fieldfalses="\"falses\":   $2"
  [ "${FAKE:-}" = nofalses ] && fieldfalses="\"falses\":null"
  echo "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"$1\",\"hits\":   $4,\"trues\":   $trues,$fieldfalses,\"first_false\":null,\"first_true\":{\"thread\":\"$thread\",\"ord\":1,\"vals\":$3}}" >&2
}
catalog() { # catalog <id> <type>
  echo "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"$1\",\"type\":\"$2\",\"must_hit\":true}" >&2
}
if [ "${FAKE:-}" != nocatalog ]; then
  if [ "${FAKE:-}" = swaptype ]; then
    catalog "$id.counter_exact" sometimes
    catalog "$id.all_started" always
  else
    catalog "$id.counter_exact" always
    catalog "$id.all_started" sometimes
  fi
fi
event "$id.all_started" 0 "[4,0,0,0]" "$hits"
event "$id.counter_exact" "$falses" "$vals" "$hits"
[ "${FAKE:-}" = incomplete ] && echo "hairspring: incomplete catalog: 1 evaluation(s) ran at call sites that are missing from the catalog section" >&2

[ "${FAKE:-}" = dupe ] && event "$id.counter_exact" 1 "[$((full - lost)),$full,4,$k]" 1
if [ "${FAKE:-}" = garbage ]; then
  echo "garbage"
elif ! printf "%s lang=%s mode=%s final=%0${digits}d expect=%020d order=0123456789abcdef rand=0123456789abcdef\n" "$head" "$lang" "$shown" "$final" "$full" 2>/dev/null; then
  if [ "${FAKE:-}" != noexit3 ]; then
    echo "fake: cannot write the result line" >&2
    exit 3
  fi
fi
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
check "a non-numeric falses counter is rejected" 1 nofalses
check "two assert lines for one id are rejected" 1 dupe
if [ -w /dev/full ]; then
  check "a program that survives an unwritable stdout with exit 0 is rejected" 1 noexit3
else
  echo "skip a program that survives an unwritable stdout is rejected (no /dev/full here)"
fi

# LANG_AT_K: the K the artifact is run at (default 1000). The expected total is 4 x K, in the line's expect= field, in the
# witness values [final, 4K, 4, K] and in the final-value comparisons.
kcheck() { # kcheck <name> <wanted exit> <FAKE value> <FAKE_K> <LANG_AT_K or "unset">
  if [ "$5" = unset ]; then
    expect "$1" "$2" env -u LANG_AT_K FAKE="$3" FAKE_K="$4" FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
  else
    expect "$1" "$2" env FAKE="$3" FAKE_K="$4" LANG_AT_K="$5" FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
  fi
}
kcheck "K=50: a conforming program run at --k=50 passes" 0 racy 50 50
kcheck "K=5000: a conforming program run at the largest K passes" 0 racy 5000 5000
kcheck "K=1: a conforming program run at K=1 passes" 0 racy 1 1
kcheck "K=50: a program still at K=1000 is rejected (expect= and witness values are 4000)" 1 racy 1000 50
kcheck "K=1000 given explicitly: the default program passes" 0 racy 1000 1000
kcheck "K=1000 given explicitly: a program that ran at K=50 is rejected" 1 racy 50 1000
kcheck "no LANG_AT_K: a program that ran at K=50 is rejected" 1 racy 50 unset
kcheck "K=50: a fixed mode that loses an update is rejected" 1 fixedfires 50 50
kcheck "K=50: a detector that disagrees with the program is rejected" 1 disagree 50 50
kcheck "K=50: witness values other than [final, 200, 4, 50] are rejected" 1 twovals 50 50
for badk in 0 abc -1 1.5 "" 050 "5 0" 5001 99999999999999999999 " " "50 "; do
  expect "LANG_AT_K='$badk' is a usage error" 64 env LANG_AT_K="$badk" FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
done
# LANG_AT_DIRECTION: up is for a fixture whose bug applies an update twice (final above 4 x K); down (the default) a lost update.
expect "direction up: a program whose racy modes apply updates twice passes" 0 env FAKE=racy FAKE_UP=1 LANG_AT_DIRECTION=up FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
expect "direction up: a program that loses updates is rejected" 1 env FAKE=racy LANG_AT_DIRECTION=up FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
expect "direction down (the default): a program that applies updates twice is rejected" 1 env FAKE=racy FAKE_UP=1 FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
expect "direction up: a fixed mode that applies an update twice is rejected" 1 env FAKE=fixedfires FAKE_UP=1 LANG_AT_DIRECTION=up FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
expect "direction up: run-checks passes with its control" 0 env FAKE=racy FAKE_UP=1 LANG_AT_DIRECTION=up FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
for baddir in "" Up sideways "up "; do
  expect "LANG_AT_DIRECTION='$baddir' is a usage error" 64 env LANG_AT_DIRECTION="$baddir" FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6
done
# The default is unchanged: with LANG_AT_K unset or 1000 the script prints the same bytes and ends the same way.
env -u LANG_AT_K FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6 >"$tmp/native.unset" 2>&1
echo "exit $?" >>"$tmp/native.unset"
env LANG_AT_K=1000 FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6 >"$tmp/native.1000" 2>&1
echo "exit $?" >>"$tmp/native.1000"
expect "check-native prints the same bytes with LANG_AT_K unset and 1000" 0 cmp -s "$tmp/native.unset" "$tmp/native.1000"
expect "check-native's default summary names no K" 0 sh -c "grep -q 'native runs' '$tmp/native.unset' && ! grep -qi 'k=' '$tmp/native.unset'"
env -u LANG_AT_K FAKE=twovals FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/check-native.sh" t "$fake" 6 >"$tmp/native.twovals" 2>&1
expect "check-native's default witness message still names [N,4000,4,1000]" 0 grep -Fq 'are not [' "$tmp/native.twovals"
expect "check-native's default witness message ends with 4000,4,1000]" 0 grep -Fq ',4000,4,1000]:' "$tmp/native.twovals"

# run-checks.sh: the real artifact, then the negative control (the fixed mode run as buggy must be rejected).
expect "run-checks passes a conforming racy program with its control" 0 env FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks treats the skip as success on one CPU" 0 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=1 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks fails a program that never fires on several CPUs" 1 env FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$fake" 6
expect "run-checks without an artifact is a usage error" 64 sh "$here/run-checks.sh" t
awkward="$tmp/dir with 'quotes' and \$dollar"
mkdir -p "$awkward"
cp "$fake" "$awkward/fake program"
expect "run-checks copes with an artifact path that needs quoting" 0 env FAKE=racy FAKE_STATE="$(newstate)" LANG_AT_CPUS=4 WIDE_RUNS=2 sh "$here/run-checks.sh" t "$awkward/fake program" 6

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
for token in HAIRSPRING_SEED '/proc/self' sysconf nproc NumCPU hardware_concurrency clock_gettime gettimeofday 'time.Now' \
  getenv os.Getenv sched_getaffinity get_nprocs 'std::chrono' ' time(0)' rdtsc runtime.GOMAXPROCS \
  available_parallelism 'Instant::now' SystemTime 'env::var'; do
  bad="$tmp/bad.c"
  printf 'void f(void) { use("%s"); }\n' "$token" >"$bad"
  expect "check-sources rejects $token" 1 sh "$here/check-sources.sh" "$bad"
done
expect "check-sources without a file is a usage error" 64 sh "$here/check-sources.sh"
expect "check-sources names the files it searched when they are clean" 0 sh -c "sh '$here/check-sources.sh' '$clean' | grep -q 'no match in:.*clean.c'"
expect "the real Rust fixture is clean" 0 sh "$here/check-sources.sh" "$here"/../../../sdks/rust/fixtures/lang_at/main.rs "$here"/../../../sdks/rust/fixtures/lang_at/window.rs "$here"/../../../sdks/rust/fixtures/lang_at/mode.rs "$here"/../../../sdks/rust/fixtures/lang_at/line.rs

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
# Realistic compiler output: one copy of the window per mode (the locked copy sits between mutex calls), the counter
# loaded before the call to the busy loop and stored, to the same address, after it.
window_ok="$tmp/window_ok.s"
cat >"$window_ok" <<'EOF'
0000000000002900 <_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode>:
    2930:	mov    %r15,%rdi
    2933:	call   2260 <pthread_mutex_lock@plt>
    2938:	test   %eax,%eax
    293a:	jne    2a40 <_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode+0x110>
    2940:	mov    0x6b82(%rip),%r12        # 9540 <counter>
    2947:	mov    0xc(%r14),%esi
    294b:	mov    %r12,%rdi
    294e:	call   2a50 <_ZL12lang_at_spinmj>
    2953:	inc    %r12
    2956:	mov    %r12,0x6b6c(%rip)        # 9540 <counter>
    295d:	mov    %r15,%rdi
    2960:	call   2140 <pthread_mutex_unlock@plt>
    2965:	jmp    297d <_ZN12_GLOBAL__N_16workerEjPK12lang_at_mode+0x7d>
    2970:	nop
    2971:	nop
    2972:	nop
    2973:	nop
    2974:	nop
    2975:	nop
    2976:	nop
    2977:	nop
    2978:	nop
    2979:	nop
    297a:	nop
    297b:	nop
    297c:	nop
    2980:	mov    0x6b59(%rip),%r12        # 9540 <counter>
    2987:	mov    0xc(%r14),%esi
    298b:	mov    %r12,%rdi
    298e:	call   2a50 <_ZL12lang_at_spinmj>
    2993:	inc    %r12
    2996:	mov    %r12,0x6b43(%rip)        # 9540 <counter>
    299d:	ret
EOF
# The compiler moved the store above the call: the update is no longer split by the busy loop.
window_store_first="$tmp/window_store_first.s"
cat >"$window_store_first" <<'EOF'
0000000000002900 <worker>:
    2980:	mov    0x6b59(%rip),%r12        # 9540 <counter>
    2987:	lea    0x1(%r12),%rax
    298b:	mov    %rax,0x6b43(%rip)        # 9540 <counter>
    2992:	mov    %r12,%rdi
    2995:	call   2a50 <lang_at_spin>
    299a:	ret
EOF
# The busy loop was hoisted out of the window: the call comes before the load and the store.
window_collapsed="$tmp/window_collapsed.s"
cat >"$window_collapsed" <<'EOF'
0000000000002900 <worker>:
    2980:	mov    $0x3e8,%esi
    2985:	call   2a50 <lang_at_spin>
    298a:	mov    0x6b59(%rip),%r12        # 9540 <counter>
    2991:	inc    %r12
    2994:	mov    %r12,0x6b43(%rip)        # 9540 <counter>
    299b:	ret
EOF
# One copy is right, the other stores to a different address.
window_one_bad="$tmp/window_one_bad.s"
cat >"$window_one_bad" <<'EOF'
0000000000002900 <worker>:
    2940:	mov    0x6b82(%rip),%r12        # 9540 <counter>
    294b:	mov    %r12,%rdi
    294e:	call   2a50 <lang_at_spin>
    2956:	mov    %r12,0x6b6c(%rip)        # 9540 <counter>
    2980:	mov    0x6b59(%rip),%r12        # 9540 <counter>
    298b:	mov    %r12,%rdi
    298e:	call   2a50 <lang_at_spin>
    2996:	mov    %r12,0x6b43(%rip)        # 9548 <other>
    299d:	ret
EOF
window_other_call="$tmp/window_other_call.s"
cat >"$window_other_call" <<'EOF'
0000000000002900 <worker>:
    2940:	mov    0x6b82(%rip),%r12        # 9540 <counter>
    294e:	call   2a50 <lang_at_spin>
    2953:	call   2100 <puts@plt>
    2956:	mov    %r12,0x6b6c(%rip)        # 9540 <counter>
    299d:	ret
EOF
# Both copies sit behind a mutex call: no unlocked path.
window_all_locked="$tmp/window_all_locked.s"
cat >"$window_all_locked" <<'EOF'
0000000000002900 <worker>:
    2933:	call   2260 <pthread_mutex_lock@plt>
    2940:	mov    0x6b82(%rip),%r12        # 9540 <counter>
    294e:	call   2a50 <lang_at_spin>
    2956:	mov    %r12,0x6b6c(%rip)        # 9540 <counter>
    299d:	ret
EOF
# Go: the store is an xchg through a pointer reloaded from the stack.
window_go="$tmp/window_go.s"
cat >"$window_go" <<'EOF'
0000000000548da0 <race.work>:
  548e13:	mov    (%rdx),%rax
  548e16:	mov    %rax,0x10(%rsp)
  548e20:	call   548f20 <race.spin>
  548e25:	mov    %rax,0x18(%rsp)
  548e2a:	mov    0x10(%rsp),%rcx
  548e2f:	inc    %rcx
  548e32:	mov    0x68(%rsp),%rdx
  548e37:	xchg   %rcx,(%rdx)
  548e3a:	ret
EOF
window_locked="$tmp/window_locked.s"
cat >"$window_locked" <<'EOF'
0000000000001300 <worker>:
    12f0:	mov    0x2f00(%rip),%r12        # 4040 <counter>
    1305:	call   1230 <lang_at_spin>
    130a:	lock xadd %rax,0x2ef0(%rip)        # 4040 <counter>
    1318:	mov    %r12,0x2ef0(%rip)        # 4040 <counter>
    1320:	ret
EOF
no_call="$tmp/no_call.s"
printf '0000000000001300 <worker>:\n    1300:\tret\n' >"$no_call"
expect "check-disasm finds the loop that survived" 0 sh "$here/check-disasm.sh" loop "$loop_ok"
expect "check-disasm reports a loop the optimiser removed" 1 sh "$here/check-disasm.sh" loop "$loop_gone"
expect "check-disasm accepts both copies of the window (load, call, store to the same address)" 0 sh "$here/check-disasm.sh" window "$window_ok"
expect "check-disasm accepts the Go window (xchg store through a reloaded pointer)" 0 sh "$here/check-disasm.sh" window "$window_go"
expect "check-disasm finds an unlocked path when one copy has no mutex call" 0 env UNLOCKED_PATH=1 sh "$here/check-disasm.sh" window "$window_ok"
expect "check-disasm rejects a program whose every copy is behind a mutex when an unlocked path is required" 1 env UNLOCKED_PATH=1 sh "$here/check-disasm.sh" window "$window_all_locked"
expect "check-disasm rejects a store hoisted above the call" 1 sh "$here/check-disasm.sh" window "$window_store_first"
expect "check-disasm rejects a busy loop hoisted out of the window" 1 sh "$here/check-disasm.sh" window "$window_collapsed"
expect "check-disasm checks every call, not the first" 1 sh "$here/check-disasm.sh" window "$window_one_bad"
expect "check-disasm rejects another call inside the window" 1 sh "$here/check-disasm.sh" window "$window_other_call"
expect "check-disasm rejects a locked read-modify-write in the window" 1 sh "$here/check-disasm.sh" window "$window_locked"
expect "check-disasm reports a worker with no call to the busy loop" 2 sh "$here/check-disasm.sh" window "$no_call"
expect "check-disasm with an unknown check is a usage error" 64 sh "$here/check-disasm.sh" other "$loop_ok"

# check-writes.sh: build-record B5, on canned `strace -f -s 4096 -e trace=write` text.
trace_ok="$tmp/trace_ok.txt"
cat >"$trace_ok" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 38
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\"}\n", 36) = 36
160   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
trace_split="$tmp/trace_split.txt"
cat >"$trace_split" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\"", 30) = 30
160   write(200, ":\"hello\"}\n", 11) = 11
160   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
trace_short="$tmp/trace_short.txt"
cat >"$trace_short" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 20
160   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
trace_two_stdout="$tmp/trace_two_stdout.txt"
cat >"$trace_two_stdout" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 38
160   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 ", 55) = 55
160   write(1, "expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 73) = 73
EOF
trace_no_stdout="$tmp/trace_no_stdout.txt"
cat >"$trace_no_stdout" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 38
EOF
# strace -f interleaves threads: a call that blocks is printed as "<unfinished ...>" and finished by "<... write resumed>".
trace_interleaved="$tmp/trace_interleaved.txt"
cat >"$trace_interleaved" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 38
161   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128 <unfinished ...>
162   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\"}\n", 36) = 36
161   <... write resumed>) = 128
EOF
trace_interleaved_split="$tmp/trace_interleaved_split.txt"
cat >"$trace_interleaved_split" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\"", 24 <unfinished ...>
161   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\"}\n", 36) = 36
160   <... write resumed>) = 24
160   write(200, ":\"hello\"}\n", 11) = 11
161   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
trace_resumed_short="$tmp/trace_resumed_short.txt"
cat >"$trace_resumed_short" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38 <unfinished ...>
160   <... write resumed>) = 20
160   write(1, "LANGAT v1 lang=c mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
expect "check-writes joins an unfinished write to its resumed half and accepts it" 0 sh "$here/check-writes.sh" "$trace_interleaved"
expect "check-writes rejects a protocol line split across two writes even when interleaved" 1 sh "$here/check-writes.sh" "$trace_interleaved_split"
expect "check-writes rejects a short write reported by the resumed half" 1 sh "$here/check-writes.sh" "$trace_resumed_short"
expect "check-writes accepts whole single-write lines" 0 sh "$here/check-writes.sh" "$trace_ok"
expect "check-writes rejects an event line split across two writes" 1 sh "$here/check-writes.sh" "$trace_split"
expect "check-writes rejects a short write" 1 sh "$here/check-writes.sh" "$trace_short"
expect "check-writes rejects a result line written in two pieces" 1 sh "$here/check-writes.sh" "$trace_two_stdout"
expect "check-writes rejects a missing result line" 1 sh "$here/check-writes.sh" "$trace_no_stdout"
expect "check-writes without a file is a usage error" 64 sh "$here/check-writes.sh"

# check-sources-python.sh: at_py.py and probe_py.py must not read the seed, /proc, the CPU count, the environment or
# the clock (the probe alone may call time.monotonic_ns).
pyclean="$tmp/py_clean.py"
printf 'import threading\nprint(1)\n' >"$pyclean"
expect "check-sources-python accepts a clean source" 0 sh "$here/check-sources-python.sh" "$pyclean"
for token in 'HAIRSPRING_SEED' '/proc/self/stat' 'os.cpu_count()' 'os.process_cpu_count()' 'os.sched_getaffinity(0)' 'os.environ["X"]' 'os.getenv("X")' 'time.time()' 'time.monotonic()' 'time.perf_counter()' 'time.process_time()' 'time.clock_gettime(0)' 'datetime.now()' 'time.monotonic_ns()'; do
  pybad="$tmp/py_bad.py"
  printf 'x = %s\n' "$token" >"$pybad"
  expect "check-sources-python rejects $token" 1 sh "$here/check-sources-python.sh" "$pybad"
done
pyclock="$tmp/py_clock.py"
printf 'import time\nt = time.monotonic_ns()\n' >"$pyclock"
expect "check-sources-python accepts time.monotonic_ns when it is allowed" 0 sh "$here/check-sources-python.sh" --allow-monotonic-ns "$pyclock"
printf 'import time\nt = time.monotonic()\n' >"$pyclock"
expect "check-sources-python still rejects time.monotonic when monotonic_ns is allowed" 1 sh "$here/check-sources-python.sh" --allow-monotonic-ns "$pyclock"
expect "check-sources-python without a file is a usage error" 64 sh "$here/check-sources-python.sh"
pyroot="$here/../python"
expect "the real at_py.py is clean" 0 sh "$here/check-sources-python.sh" "$pyroot/at_py.py"
expect "the real probe_py.py is clean, apart from monotonic_ns" 0 sh "$here/check-sources-python.sh" --allow-monotonic-ns "$pyroot/probe_py.py"
expect "the real probe_py.py is rejected when monotonic_ns is not allowed" 1 sh "$here/check-sources-python.sh" "$pyroot/probe_py.py"

# stage-python.sh: the staged package (the fixture, the probe and the SDK package) with the sha256 of every file and a
# tree hash, so two stagings of the same sources compare equal and a changed byte does not.
stage1="$tmp/stage1"
stage2="$tmp/stage2"
expect "stage-python stages the package" 0 sh "$here/stage-python.sh" "$stage1" "$tmp/list1.txt"
expect "stage-python stages it again elsewhere" 0 sh "$here/stage-python.sh" "$stage2" "$tmp/list2.txt"
if cmp -s "$tmp/list1.txt" "$tmp/list2.txt"; then echo "ok   two stagings have equal listings and tree hash"; else echo "FAIL two stagings differ"; failures=$((failures + 1)); fi
expect "the listing names the fixture, the probe and the package" 0 sh -c "grep -q ' at_py.py\$' '$tmp/list1.txt' && grep -q ' probe_py.py\$' '$tmp/list1.txt' && grep -q ' hairspring/__init__.py\$' '$tmp/list1.txt' && grep -q ' hairspring/_runtime.py\$' '$tmp/list1.txt'"
expect "the listing ends with a tree hash" 0 sh -c "tail -1 '$tmp/list1.txt' | grep -Eq '^tree  [0-9a-f]{64}\$'"
expect "the listing has no bytecode" 1 grep -q pycache "$tmp/list1.txt"
cp -R "$here/../python" "$tmp/fx_copy"
mkdir -p "$tmp/sdk_copy"
cp -R "$here/../../../sdks/python/src/." "$tmp/sdk_copy"
printf '# changed\n' >>"$tmp/sdk_copy/hairspring/_fmt.py"
expect "stage-python stages changed sources" 0 env FIXTURE_DIR="$tmp/fx_copy" SDK_SRC="$tmp/sdk_copy" sh "$here/stage-python.sh" "$tmp/stage3" "$tmp/list3.txt"
if [ "$(tail -1 "$tmp/list1.txt")" != "$(tail -1 "$tmp/list3.txt")" ]; then echo "ok   a changed byte changes the tree hash"; else echo "FAIL a changed byte did not change the tree hash"; failures=$((failures + 1)); fi
expect "stage-python without arguments is a usage error" 64 sh "$here/stage-python.sh"

# check-evidence.sh: the evidence a build record needs for checklist lines 2, 4, 6, 7, 8 and 9, captured from the artifact.
evidence="$here/check-evidence.sh"
# sample <FAKE value> <mode> [env...]: captures one run of the fake and prints the prefix of the captured files.
sample() {
  behaviour=$1
  mode=$2
  shift 2
  prefix="$(newstate)/run"
  env FAKE="$behaviour" FAKE_STATE="$(newstate)" "$@" sh "$evidence" capture "$fake" "$mode" "$prefix" >/dev/null 2>&1
  echo "$prefix"
}
expect "capture records stdout, stderr and the exit status" 0 sh -c "p=\$(env FAKE_STATE='$(newstate)' sh '$evidence' capture '$fake' buggy '$tmp/cap' >/dev/null 2>&1; echo '$tmp/cap'); test -s \$p.out && test -s \$p.err && test \"\$(cat \$p.rc)\" = 0"
expect "capture of a missing artifact fails" 1 sh "$evidence" capture "$tmp/no-such-program" buggy "$tmp/cap-missing"
expect "evidence without a check is a usage error" 64 sh "$evidence"
expect "evidence with an unknown check is a usage error" 64 sh "$evidence" other

# line 2: a bad argument and no argument, each a usage line on stderr, empty stdout, exit 64
expect "usage evidence passes for a conforming program" 0 env FAKE=racy FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence rejects exit 2 instead of 64" 1 env FAKE=racy FAKE_USAGE_STATUS=2 FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence rejects a program that ignores a bad argument" 1 env FAKE=nousage FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence rejects a usage line on stdout" 1 env FAKE=usageout FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence shows both runs and the exit status" 0 sh -c "env FAKE=racy FAKE_STATE='$(newstate)' sh '$evidence' usage t '$fake' | grep -c 'exit status: 64' | grep -qx 3"
expect "usage evidence rejects stderr with protocol lines but no usage line" 1 env FAKE=nousagetext FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence finds the usage line among protocol lines" 0 env FAKE=usageevents FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence shows the usage line itself" 0 sh -c "env FAKE=usageevents FAKE_STATE='$(newstate)' sh '$evidence' usage t '$fake' | grep -q '^usage line: usage: fake buggy|fixed|wide'"

# line 4: the start-barrier source excerpt
barrier_c="$tmp/barrier.c"
printf 'static unsigned started;\nstatic int wait_all(void) {\n  while (__atomic_load_n(&started, __ATOMIC_ACQUIRE) < LANG_AT_THREADS) {\n  }\n}\nvoid w(void) {\n  __atomic_fetch_add(&started, 1u, __ATOMIC_ACQ_REL);\n  wait_all();\n}\n' >"$barrier_c"
barrier_rust="$tmp/barrier.rs"
printf 'fn wait_for_all(&self) {\n    while self.started.load(SeqCst) < THREADS {\n    }\n}\nfn w(&self) {\n    self.started.fetch_add(1, SeqCst);\n    self.wait_for_all();\n}\n' >"$barrier_rust"
barrier_go="$tmp/barrier.go"
printf 'st.started.Add(1)\nfor st.started.Load() < Workers {\n}\n' >"$barrier_go"
barrier_py="$tmp/barrier.py"
printf 'with lock:\n    shared.started += 1\nshared.barrier.wait()\n' >"$barrier_py"
barrier_none="$tmp/barrier_none.c"
printf 'void w(void) { run(); }\n' >"$barrier_none"
barrier_inc_only="$tmp/barrier_inc.c"
printf 'void w(void) { __atomic_fetch_add(&started, 1u, __ATOMIC_ACQ_REL); }\n' >"$barrier_inc_only"
barrier_wait_only="$tmp/barrier_wait.c"
printf 'void w(void) { while (__atomic_load_n(&started, __ATOMIC_ACQUIRE) < LANG_AT_THREADS) {} }\n' >"$barrier_wait_only"
expect "barrier evidence accepts the C increment and wait" 0 sh "$evidence" barrier "$barrier_c"
expect "barrier evidence accepts the Rust increment and wait" 0 sh "$evidence" barrier "$barrier_rust"
expect "barrier evidence accepts the Go increment and wait" 0 sh "$evidence" barrier "$barrier_go"
expect "barrier evidence accepts the Python increment and Barrier.wait" 0 sh "$evidence" barrier "$barrier_py"
expect "barrier evidence rejects a source with no barrier" 1 sh "$evidence" barrier "$barrier_none"
expect "barrier evidence rejects an increment with no wait" 1 sh "$evidence" barrier "$barrier_inc_only"
expect "barrier evidence rejects a wait with no increment" 1 sh "$evidence" barrier "$barrier_wait_only"
expect "barrier evidence rejects the increment and the wait in different files" 1 sh "$evidence" barrier "$barrier_inc_only" "$barrier_wait_only"
# A comparison of started inside the sometimes() call, or in a comment, is not a barrier.
barrier_assert_only="$tmp/barrier_assert.c"
printf 'void w(void) {\n  __atomic_fetch_add(&started, 1u, __ATOMIC_ACQ_REL);\n  run();\n}\nvoid m(void) {\n  HS_SOMETIMES(started == LANG_AT_THREADS, "c.all_started", "x", started);\n  /* while (started < LANG_AT_THREADS) wait */\n  // while (started < LANG_AT_THREADS) wait\n}\n' >"$barrier_assert_only"
expect "barrier evidence rejects a wait that is only in an assertion call or a comment" 1 sh "$evidence" barrier "$barrier_assert_only"
barrier_py_assert="$tmp/barrier_assert.py"
printf 'with lock:\n    shared.started += 1\nrun()\nhs.sometimes(shared.started == THREADS, "py.all_started", "x", shared.started)\n# shared.barrier.wait()\n' >"$barrier_py_assert"
expect "barrier evidence rejects a Python wait that is only an assertion or a comment" 1 sh "$evidence" barrier "$barrier_py_assert"
barrier_far="$tmp/barrier_far.c"
printf 'void w(void) {\n  __atomic_fetch_add(&started, 1u, __ATOMIC_ACQ_REL);\n  a();\n  b();\n  c();\n  d();\n  e();\n  f();\n  g();\n}\nvoid other(void) {\n  while (__atomic_load_n(&started, __ATOMIC_ACQUIRE) < LANG_AT_THREADS) {}\n}\n' >"$barrier_far"
expect "barrier evidence rejects a wait in another function that the worker does not call" 1 sh "$evidence" barrier "$barrier_far"
# Deleting the wait from a copy of each real fixture must not pass.
nowait() { # nowait <name> <source> <line pattern of the wait>: a copy without the wait lines
  grep -v "$3" "$2" >"$tmp/nowait.$1"
  echo "$tmp/nowait.$1"
}
fixtures_dir="$here/.."
expect "the C fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait c "$fixtures_dir/c/src/lost_update.c" '__atomic_load_n(&started')"
expect "the C++ fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait cc "$fixtures_dir/cpp/src/lost_update.cc" 'started.load(std::memory_order_acquire) <')"
expect "the Go fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait go "$fixtures_dir/go/internal/race/race.go" 'started.Load() < Workers')"
expect "the Python fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait py "$fixtures_dir/python/at_py.py" 'barrier.wait()')"
expect "the Rust fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait rs "$fixtures_dir/../../sdks/rust/fixtures/lang_at/window.rs" 'started.load(SeqCst) < THREADS')"
expect "barrier evidence without a file is a usage error" 64 sh "$evidence" barrier
expect "barrier evidence prints the matching lines with line numbers" 0 sh -c "sh '$evidence' barrier '$barrier_c' | grep -q 'fetch_add(&started'"
expect "the real C fixture has a start barrier" 0 sh "$evidence" barrier "$here/../c/src/lost_update.c"
expect "the real C++ fixture has a start barrier" 0 sh "$evidence" barrier "$here/../cpp/src/lost_update.cc"
expect "the real Go fixture has a start barrier" 0 sh "$evidence" barrier "$here/../go/internal/race/race.go"
expect "the real Python fixture has a start barrier" 0 sh "$evidence" barrier "$here/../python/at_py.py"
expect "the real Rust fixture has a start barrier" 0 sh "$evidence" barrier "$here/../../../sdks/rust/fixtures/lang_at/window.rs"

# lines 6 and 7: the always and sometimes event lines of one native run
expect "witness evidence passes for a conforming run" 0 sh "$evidence" witness t "$(sample racy buggy)"
expect "witness evidence passes for a fixed run" 0 sh "$evidence" witness t "$(sample racy fixed)"
expect "witness evidence rejects two witness values" 1 sh "$evidence" witness t "$(sample twovals buggy)"
expect "witness evidence rejects a property evaluated twice" 1 sh "$evidence" witness t "$(sample twice buggy)"
expect "witness evidence rejects the wrong id prefix" 1 sh "$evidence" witness t "$(sample wrongid buggy)"
expect "witness evidence rejects swapped catalog types" 1 sh "$evidence" witness t "$(sample swaptype buggy)"
expect "witness evidence rejects a missing catalog" 1 sh "$evidence" witness t "$(sample nocatalog buggy)"
expect "witness evidence rejects two assert lines for one id" 1 sh "$evidence" witness t "$(sample dupe buggy)"
expect "witness evidence prints the always line with the witness values" 0 sh -c "sh '$evidence' witness t '$(sample racy fixed)' | grep -q '\"vals\":\[4000,4000,4,1000\]'"
expect "witness evidence states the witness values it found" 0 sh -c "sh '$evidence' witness t '$(sample racy fixed)' | grep -q '^ok .*\[4000, 4000, 4, 1000\]'"
expect "witness evidence names the catalog types" 0 sh -c "sh '$evidence' witness t '$(sample racy fixed)' | grep -q '\"type\":\"always\"'"
expect "ids evidence passes for the two contract ids" 0 sh "$evidence" ids t "$(sample racy buggy)"
expect "ids evidence rejects the cpp prefix for C++" 1 sh "$evidence" ids cxx "$(sample wrongid buggy FAKE_LANG=cxx)"
expect "ids evidence accepts the cxx prefix for C++" 0 sh "$evidence" ids cxx "$(sample racy buggy FAKE_LANG=cxx)"
expect "ids evidence lists the ids" 0 sh -c "sh '$evidence' ids t '$(sample racy buggy)' | grep -q 't.counter_exact' "

# line 8: one sample stdout line
expect "stdout-line evidence passes for the contract line" 0 sh "$evidence" stdout-line t buggy "$(sample racy buggy)"
expect "stdout-line evidence rejects the old line" 1 sh "$evidence" stdout-line t buggy "$(sample oldline buggy)"
expect "stdout-line evidence rejects an extra line" 1 sh "$evidence" stdout-line t buggy "$(sample extraline buggy)"
expect "stdout-line evidence rejects a line that names the wrong mode" 1 sh "$evidence" stdout-line t fixed "$(sample wrongmode fixed)"
expect "stdout-line evidence rejects a short final" 1 sh "$evidence" stdout-line t buggy "$(sample shortfinal buggy)"
expect "stdout-line evidence prints the line" 0 sh -c "sh '$evidence' stdout-line t buggy '$(sample racy buggy)' | grep -q '^LANGAT v1 lang=t mode=buggy '"

# line 9: exit codes 0, 3 and 64
if [ -w /dev/full ]; then
  expect "exit-code evidence passes for a conforming program" 0 env FAKE=racy FAKE_STATE="$(newstate)" sh "$evidence" exit-codes t "$fake"
  expect "exit-code evidence rejects a program that survives an unwritable stdout" 1 env FAKE=noexit3 FAKE_STATE="$(newstate)" sh "$evidence" exit-codes t "$fake"
else
  echo "skip exit-code evidence (no /dev/full here)"
fi
expect "exit-code evidence rejects a non-zero exit from a normal run" 1 env FAKE=badexit FAKE_STATE="$(newstate)" sh "$evidence" exit-codes t "$fake"
expect "exit-code evidence rejects usage status 2" 1 env FAKE=racy FAKE_USAGE_STATUS=2 FAKE_STATE="$(newstate)" sh "$evidence" exit-codes t "$fake"

# record-lib.sh: the shared pieces of the build-record scripts (steps, verdicts, ticked checklist lines, producer fields)
libcase() { # libcase <name> <wanted exit> <snippet run after sourcing the library with a fresh work directory>
  lname=$1
  lwanted=$2
  lsnippet=$3
  expect "$lname" "$lwanted" sh -c '. "$1/record-lib.sh"; work=$(mktemp -d "$2/w.XXXXXX"); out=$work/record.md; record_init; '"$lsnippet" lib "$here" "$tmp"
}
libcase "a passing step has the verdict PASS" 0 'step B1 t true; [ "$(verdict B1)" = PASS ]'
libcase "a failing step has the verdict FAIL and sets the failed flag" 0 'step B1 t false; [ "$(verdict B1)" = FAIL ] && [ "$failed" = 1 ]'
libcase "one failing step among several makes the id FAIL" 0 'step B1 a true; step B1 b false; step B1 c true; [ "$(verdict B1)" = FAIL ]'
libcase "a step that was not run has the verdict NOT-RUN and does not pass" 0 'not_run B5 t why; [ "$(verdict B5)" = NOT-RUN ]'
libcase "a check that does not apply passes" 0 'not_applicable B3 t why; [ "$(verdict B3)" = PASS ]'
libcase "an id with no step is MISSING" 0 '[ "$(verdict B9)" = MISSING ]'
libcase "a step records the command and its output in the record" 0 'step B1 "the title" echo hello; grep -q "^### B1 the title: PASS" "$body" && grep -q "^hello" "$body"'
libcase "a ticked line names the evidence that proves it" 0 'step B1 t true; tickline 1 "One artifact." B1; grep -q "^- \[x\] 1. One artifact. (proved by B1)" "$body"'
libcase "a failing proof leaves the line unticked and shows the failure" 0 'step B1 t false; tickline 1 "One artifact." B1; grep -q "^- \[ \] 1. One artifact. (not ticked: B1 FAIL)" "$body"'
libcase "missing evidence leaves the line unticked" 0 'tickline 2 "Usage." L2; grep -q "^- \[ \] 2. Usage. (not ticked: L2 MISSING)" "$body"'
libcase "only the failing ids are listed" 0 'step B1 a true; step B2 b false; not_run B5 c why; tickline 3 "x" B1 B2 B5; grep -q "(not ticked: B2 FAIL, B5 NOT-RUN)" "$body"'
libcase "a line needing several ids is ticked when all pass" 0 'step B1 a true; step B7 b true; not_applicable B3 c why; tickline 3 "x" B1 B3 B7; grep -q "^- \[x\] 3. x (proved by B1, B3 (N/A), B7)" "$body"'
libcase "producer fields from a GitHub Actions run" 0 'GITHUB_RUN_ID=12345 GITHUB_SERVER_URL=https://github.com GITHUB_REPOSITORY=o/r GITHUB_WORKFLOW_REF=o/r/.github/workflows/ci.yml@refs/heads/b GITHUB_WORKFLOW_SHA=abc123 ImageOS=ubuntu24 ImageVersion=20261018.339.1 SOURCE_COMMIT=def456 PRODUCER="GitHub Actions" producer_block >"$work/p"; grep -q "^GitHub run: 12345 (https://github.com/o/r/actions/runs/12345)" "$work/p" && grep -q "^Workflow file: .github/workflows/ci.yml, run at commit .abc123." "$work/p" && grep -q "^Runner image: ubuntu24 20261018.339.1" "$work/p" && grep -q "^Source commit: .def456. (the commit the artifacts were built from)" "$work/p" && ! grep -q "Source tree" "$work/p"'
libcase "producer fields outside GitHub say so" 0 'unset GITHUB_RUN_ID GITHUB_WORKFLOW_REF GITHUB_WORKFLOW_SHA ImageOS ImageVersion; SOURCE_COMMIT=def456 producer_block >"$work/p"; grep -q "^GitHub run: <TO BE FILLED" "$work/p" && grep -q "^Runner image: <TO BE FILLED" "$work/p" && grep -q "^Source commit: .def456." "$work/p"'
libcase "publishing an artifact copies it with a SHA256SUMS line and prints its sha256" 0 'printf x >"$work/bin"; ARTIFACT_DIR=$work/up; export ARTIFACT_DIR; sha=$(publish_artifact "$work/bin" at-x); [ "$sha" = 2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881 ] && cmp -s "$work/bin" "$work/up/at-x" && grep -q "^$sha  at-x$" "$work/up/SHA256SUMS"'
libcase "publishing without an artifact directory still prints the sha256" 0 'printf x >"$work/bin"; unset ARTIFACT_DIR; sha=$(publish_artifact "$work/bin" at-x); [ "$sha" = 2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881 ]'
libcase "publishing a missing file fails" 1 'unset ARTIFACT_DIR; publish_artifact "$work/none" at-x'
libcase "publishing a staged tree copies the files; SHA256SUMS lists them and checks with sha256sum -c, the tree hash is in a file of its own" 0 'mkdir -p "$work/tree/sub"; printf a >"$work/tree/a"; printf b >"$work/tree/sub/b"; ARTIFACT_DIR=$work/up; export ARTIFACT_DIR; publish_tree "$work/tree" pkg abc >/dev/null; cmp -s "$work/tree/sub/b" "$work/up/pkg/sub/b" && ! grep -q "^tree" "$work/up/SHA256SUMS" && grep -q "^3e23e8160039594a33894f6564e1b1348bbd7a0088d42c4acb73eeaed59c009d  pkg/sub/b$" "$work/up/SHA256SUMS" && [ "$(cat "$work/up/pkg.tree")" = abc ] && cd "$work/up" && if command -v sha256sum >/dev/null; then sha256sum -c SHA256SUMS; else shasum -a 256 -c SHA256SUMS; fi'
libcase "starting a record clears an old SHA256SUMS in the artifact directory" 0 'mkdir -p "$work/up"; echo old >"$work/up/SHA256SUMS"; ARTIFACT_DIR=$work/up; export ARTIFACT_DIR; record_init; ! grep -q old "$work/up/SHA256SUMS"'
libcase "equal hashes pass" 0 'equal_hashes at-x aa aa'
libcase "different hashes fail" 1 'equal_hashes at-x aa bb'
libcase "an empty first hash (no binary) fails" 1 'equal_hashes at-x "" ""'
libcase "an empty second hash fails" 1 'equal_hashes at-x aa ""'
libcase "empty output to show fails" 1 'printf "" | require_lines'
libcase "non-empty output to show passes and is repeated" 0 'printf "x\n" | require_lines | grep -qx x'
libcase "a check that does not apply is labelled N/A, not PASS" 0 'not_applicable B3 t why; [ "$(verdict_label B3)" = N/A ]'
libcase "a check with a passing step and a part that does not apply is labelled PASS+N/A" 0 'not_applicable B2 t why; step B2 t true; [ "$(verdict_label B2)" = PASS+N/A ]'
libcase "a plain pass is labelled PASS" 0 'step B1 t true; [ "$(verdict_label B1)" = PASS ]'
libcase "the verdicts line says N/A for a check that does not apply" 0 'step B1 t true; not_applicable B3 t why; record_title=t; record_generated_by=g; finish_record >/dev/null; grep -q "^Verdicts: B1 PASS, B3 N/A" "$out"'
libcase "a check that does not apply and needs a review statement says it is not yet made" 0 'not_applicable B3 t why yes; grep -q "Required by the spec and not yet made \[I\]" "$body"'
libcase "a check that does not apply and needs no review statement does not say that" 0 'not_applicable B2 t why; ! grep -q "not yet made" "$body"'
libcase "line 10 shows N/A in its proved-by text" 0 'lang=t; for i in B1 B2 B4 B5 B6 B7; do step $i t true; done; not_applicable B3 t why yes; alignment_checklist a b c; grep -q "^- \[x\] 10\. .*B3 (N/A)" "$body"'
libcase "line 10 names the review statements that are required and not yet made" 0 'lang=t; for i in B1 B2 B4 B5 B6 B7; do step $i t true; done; not_applicable B3 t why yes; alignment_checklist a b c; grep -q "^- \[x\] 10\. .*not checked by the tool: .*code-review statement" "$body"'
libcase "line 6 is not ticked when the stdout line check (L8) failed" 0 'lang=t; step L6 a true; step B4 a true; step L8 a false; alignment_checklist a b c; grep -q "^- \[ \] 6\. .*(not ticked: L8 FAIL)" "$body"'
libcase "line 6 is not ticked when the native runs (B4) failed" 0 'lang=t; step L6 a true; step L8 a true; step B4 a false; alignment_checklist a b c; grep -q "^- \[ \] 6\. .*(not ticked: B4 FAIL)" "$body"'
libcase "line 3 needs the constants check" 0 'lang=t; step B7 a true; step B4 a true; alignment_checklist a b c; grep -q "^- \[ \] 3\. .*(not ticked: L3 MISSING)" "$body"'
libcase "lines 1, 3, 4 and 6 state what the tool does not check" 0 'lang=t; alignment_checklist a b c; grep -q "^- \[.\] 1\. .*not checked by the tool" "$body" && grep -q "^- \[.\] 3\. .*not checked by the tool" "$body" && grep -q "^- \[.\] 4\. .*not checked by the tool" "$body" && grep -q "^- \[.\] 6\. .*not checked by the tool" "$body"'
libcase "line 4 names its known false accept" 0 'lang=t; alignment_checklist a b c; grep -q "^- \[.\] 4\. .*if (started == 4) log();" "$body"'
libcase "line 3 says it checks constant values, not their use" 0 'lang=t; alignment_checklist a b c; grep -q "^- \[.\] 3\. .*constant values.*not that every mode uses them" "$body"'
libcase "line 6 says exactly what the thread check is" 0 'lang=t; alignment_checklist a b c; grep -q "^- \[.\] 6\. .*key.*t#5.*Go: go" "$body"'
libcase "line 9 needs the usage evidence as well" 0 'lang=t; step L9 a true; step B4 a true; alignment_checklist a b c; grep -q "^- \[ \] 9\. .*L2 MISSING" "$body"'

# The source commit is labelled +dirty when the tree has uncommitted changes.
gitrepo="$tmp/gitrepo"
mkdir -p "$gitrepo"
git -C "$gitrepo" init -q
printf 'x\n' >"$gitrepo/f"
git -C "$gitrepo" add f
git -C "$gitrepo" -c user.email=t@example.com -c user.name=t commit -q -m first
expect "a clean tree gives the plain commit" 0 sh -c ". '$here/record-lib.sh'; c=\$(source_commit_of '$gitrepo'); [ \"\$c\" = \"\$(git -C '$gitrepo' rev-parse HEAD)\" ]"
printf 'y\n' >>"$gitrepo/f"
expect "a modified tracked file gives a +dirty commit" 0 sh -c ". '$here/record-lib.sh'; c=\$(source_commit_of '$gitrepo'); [ \"\$c\" = \"\$(git -C '$gitrepo' rev-parse HEAD)+dirty\" ]"
git -C "$gitrepo" checkout -q f
printf 'z\n' >"$gitrepo/untracked"
expect "an untracked file gives a +dirty commit" 0 sh -c ". '$here/record-lib.sh'; c=\$(source_commit_of '$gitrepo'); [ \"\$c\" = \"\$(git -C '$gitrepo' rev-parse HEAD)+dirty\" ]"
expect "a directory that is not a repository gives unknown" 0 sh -c ". '$here/record-lib.sh'; [ \"\$(source_commit_of '$tmp')\" = unknown ]"

# line 3: the constants T = 4, K = 1000, W = 1000, W_wide = 10000 are asserted per language
expect "constants evidence passes for the C header" 0 sh "$evidence" constants c "$fixtures_dir/common/lang_at.h"
expect "constants evidence passes for the C++ header" 0 sh "$evidence" constants cxx "$fixtures_dir/common/lang_at.h"
expect "constants evidence passes for the Go fixture" 0 sh "$evidence" constants go "$fixtures_dir/go/internal/race/race.go"
expect "constants evidence passes for the Rust fixture" 0 sh "$evidence" constants rust "$fixtures_dir/../../sdks/rust/fixtures/lang_at/mode.rs"
expect "constants evidence passes for the Python fixture" 0 sh "$evidence" constants py "$fixtures_dir/python/at_py.py"
sed 's/LANG_AT_SPIN 1000u/LANG_AT_SPIN 2000u/' "$fixtures_dir/common/lang_at.h" >"$tmp/lang_at_w2000.h"
expect "a C window of 2000 is rejected" 1 sh "$evidence" constants c "$tmp/lang_at_w2000.h"
sed 's/LANG_AT_ITERS 1000u/LANG_AT_ITERS 999u/' "$fixtures_dir/common/lang_at.h" >"$tmp/lang_at_k999.h"
expect "a C iteration count of 999 is rejected" 1 sh "$evidence" constants c "$tmp/lang_at_k999.h"
sed 's/\(Spin  *= *\)1000 /\12000 /' "$fixtures_dir/go/internal/race/race.go" >"$tmp/race_w2000.go"
expect "a Go window of 2000 is rejected" 1 sh "$evidence" constants go "$tmp/race_w2000.go"
sed 's/WINDOW: u64 = 1_000/WINDOW: u64 = 2_000/' "$fixtures_dir/../../sdks/rust/fixtures/lang_at/mode.rs" >"$tmp/mode_w2000.rs"
expect "a Rust window of 2000 is rejected" 1 sh "$evidence" constants rust "$tmp/mode_w2000.rs"
sed 's/WINDOW_WIDE: u64 = 10_000/WINDOW_WIDE: u64 = 20_000/' "$fixtures_dir/../../sdks/rust/fixtures/lang_at/mode.rs" >"$tmp/mode_wide.rs"
expect "a Rust wide window of 20000 is rejected" 1 sh "$evidence" constants rust "$tmp/mode_wide.rs"
sed 's/^WINDOW = 1000/WINDOW = 2000/' "$fixtures_dir/python/at_py.py" >"$tmp/at_py_w2000.py"
expect "a Python window of 2000 is rejected" 1 sh "$evidence" constants py "$tmp/at_py_w2000.py"
expect "constants evidence rejects a file without the constants" 1 sh "$evidence" constants rust "$clean"
expect "constants evidence without a file is a usage error" 64 sh "$evidence" constants rust

# line 6: witness values need a parsable final in stdout, and the evaluating thread is not a worker
expect "witness evidence rejects garbage stdout even when the vals look plausible" 1 sh "$evidence" witness t "$(sample garbage buggy)"
expect "witness evidence rejects an assertion evaluated on a worker thread" 1 sh "$evidence" witness t "$(sample workerthread buggy)"
expect "witness evidence prints the evaluating thread" 0 sh -c "sh '$evidence' witness t '$(sample racy buggy)' | grep -q 'evaluated on thread t#5'"
expect "witness evidence rejects an assertion evaluated on a worker's claim-order key (t#2)" 1 sh "$evidence" witness t "$(sample workerkey buggy)"
expect "witness evidence rejects each worker key t#1 to t#4" 0 sh -c "for k in 1 2 3 4; do sed 's/\"thread\":\"t#5\"/\"thread\":\"t#'\$k'\"/' '$(sample racy buggy).err' > '$tmp/k.err'; cp '$(sample racy buggy).out' '$tmp/k.out'; sh '$evidence' witness t '$tmp/k' >/dev/null 2>&1 && exit 1; done; exit 0"
expect "witness evidence accepts the Go main thread key (go)" 0 sh "$evidence" witness go "$(sample racy buggy FAKE_LANG=go)"
expect "witness evidence rejects a claim-order key for Go" 1 sh "$evidence" witness go "$(sample workerkey buggy FAKE_LANG=go)"

# line 2: extra arguments are rejected for every language, Python alone accepts --switch=1.0
expect "usage evidence rejects a program that accepts an extra argument" 1 env FAKE=extraok FAKE_STATE="$(newstate)" sh "$evidence" usage t "$fake"
expect "usage evidence shows the extra-argument run" 0 sh -c "env FAKE=racy FAKE_STATE='$(newstate)' sh '$evidence' usage t '$fake' | grep -q 'buggy extra'"
expect "switch evidence passes for a program that accepts exactly --switch=1.0" 0 env FAKE=switchok FAKE_STATE="$(newstate)" sh "$evidence" switch t "$fake"
expect "switch evidence rejects a program that does not accept --switch=1.0" 1 env FAKE=racy FAKE_STATE="$(newstate)" sh "$evidence" switch t "$fake"
expect "switch evidence rejects a program that accepts any second argument" 1 env FAKE=switchany FAKE_STATE="$(newstate)" sh "$evidence" switch t "$fake"
if command -v python3 >/dev/null 2>&1; then
  stage_py="$tmp/stage_py"
  sh "$here/stage-python.sh" "$stage_py" "$tmp/list_py.txt" >/dev/null 2>&1
  printf '#!/bin/sh\nexec python3 -I -B "%s/at_py.py" "$@"\n' "$stage_py" >"$tmp/at-py"
  chmod +x "$tmp/at-py"
  expect "the real Python fixture passes the usage evidence (extra arguments rejected)" 0 sh "$evidence" usage py "$tmp/at-py"
  expect "the real Python fixture accepts --switch=1.0 and rejects --switch=2 and extra arguments" 0 sh "$evidence" switch py "$tmp/at-py"
else
  echo "skip the real Python usage and switch evidence (no python3)"
fi

# line 11: a missing or unreadable source file fails, directories are searched whole, more patterns are forbidden
expect "check-sources fails on a file that does not exist" 1 sh "$here/check-sources.sh" /nonexistent/file.c
expect "check-sources-python fails on a file that does not exist" 1 sh "$here/check-sources-python.sh" /nonexistent/file.py
expect "check-sources fails when one of two files is missing" 1 sh "$here/check-sources.sh" "$clean" /nonexistent/file.c
srcdir="$tmp/srcdir"
mkdir -p "$srcdir/sub"
printf 'int f(void) { return 0; }\n' >"$srcdir/a.c"
printf 'package x\n' >"$srcdir/sub/b.go"
expect "check-sources searches every file of a directory" 0 sh "$here/check-sources.sh" "$srcdir"
printf 'void g(void) { getenv("X"); }\n' >"$srcdir/sub/c.rs"
expect "check-sources finds a forbidden read in a file below a directory" 1 sh "$here/check-sources.sh" "$srcdir"
mkdir -p "$tmp/emptydir"
expect "check-sources fails on a directory with no sources" 1 sh "$here/check-sources.sh" "$tmp/emptydir"
printf 'package x\nfunc f() { time.Now() }\n' >"$srcdir/sub/d_test.go"
rm -f "$srcdir/sub/c.rs"
expect "check-sources leaves Go test files out of a directory search" 0 sh "$here/check-sources.sh" "$srcdir"
for token in 'os.LookupEnv' 'os.Environ()' 'time.Since' 'clock()' 'timespec_get'; do
  bad="$tmp/bad2.c"
  printf 'void f(void) { use("%s"); }\n' "$token" >"$bad"
  expect "check-sources rejects $token" 1 sh "$here/check-sources.sh" "$bad"
done

# the build record scripts' own helpers: a verification of the artifact as downloaded
dl="$tmp/dl"
mkdir -p "$dl/fixture-x"
printf 'binary-bytes' >"$dl/fixture-x/at-x"
dlsha=$(printf 'binary-bytes' | { if command -v sha256sum >/dev/null; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1)
printf '%s  at-x\n' "$dlsha" >"$dl/fixture-x/SHA256SUMS"
printf '# BUILD-RECORD: at-x\n\nArtifact: at-x, sha256 %s\n' "$dlsha" >"$dl/record.md"
expect "verify-download accepts an artifact whose bytes match SHA256SUMS and the record" 0 sh "$here/verify-download.sh" "$dl/fixture-x" "$dl/record.md"
printf 'binary-bytez' >"$dl/fixture-x/at-x"
expect "verify-download rejects changed bytes" 1 sh "$here/verify-download.sh" "$dl/fixture-x" "$dl/record.md"
printf 'binary-bytes' >"$dl/fixture-x/at-x"
printf '# BUILD-RECORD: at-x\n\nArtifact: at-x, sha256 %s\n' "0000000000000000000000000000000000000000000000000000000000000000" >"$dl/record-bad.md"
expect "verify-download rejects a record that names another sha256" 1 sh "$here/verify-download.sh" "$dl/fixture-x" "$dl/record-bad.md"
expect "verify-download rejects a record with no artifact line" 1 sh "$here/verify-download.sh" "$dl/fixture-x" "$clean"
expect "verify-download without arguments is a usage error" 64 sh "$here/verify-download.sh"
expect "verify-download rejects a missing directory" 1 sh "$here/verify-download.sh" "$dl/none" "$dl/record.md"
# a package: per-file lines in SHA256SUMS, the tree hash in a file of its own
pk="$tmp/pk"
mkdir -p "$pk/art"
cp -R "$stage_py" "$pk/art/at-py" 2>/dev/null
treehash=$(tail -1 "$tmp/list_py.txt" | awk '{print $2}')
if [ -d "$pk/art/at-py" ] && [ -n "$treehash" ]; then
  (cd "$pk/art" && find at-py -type f | LC_ALL=C sort | while read -r f; do printf '%s  %s\n' "$(if command -v sha256sum >/dev/null; then sha256sum "$f"; else shasum -a 256 "$f"; fi | cut -d' ' -f1)" "$f"; done >SHA256SUMS)
  printf '%s\n' "$treehash" >"$pk/art/at-py.tree"
  printf '# BUILD-RECORD: p\n\nArtifact: at-py, tree hash (x) %s\n' "$treehash" >"$pk/record.md"
  expect "verify-download recomputes the tree hash of a package and accepts it" 0 sh "$here/verify-download.sh" "$pk/art" "$pk/record.md"
  printf '# changed\n' >>"$pk/art/at-py/at_py.py"
  expect "verify-download rejects a package with a changed file" 1 sh "$here/verify-download.sh" "$pk/art" "$pk/record.md"
fi

# check-disasm: with the mutex inlined (a lock cmpxchg), an unlocked copy of the window must still exist
window_inline_locked="$tmp/window_inline_locked.s"
cat >"$window_inline_locked" <<'EOF'
0000000000002900 <worker>:
    2910:	lock cmpxchg %ecx,(%r14)
    2915:	mov    0x20(%r12),%r13
    291a:	mov    $0x3e8,%edi
    291f:	call   2a50 <spin>
    2924:	inc    %r13
    2927:	mov    %r13,0x20(%r12)
    292c:	xchg   %eax,(%r14)
    2930:	ret
EOF
window_inline_both="$tmp/window_inline_both.s"
{
  cat "$window_inline_locked"
  i=0
  while [ "$i" -lt 10 ]; do printf '    2940:\tnop\n'; i=$((i + 1)); done
  printf '    2950:\tmov    0x20(%%r12),%%r15\n    2955:\tmov    $0x3e8,%%edi\n    295a:\tcall   2a50 <spin>\n    295f:\tinc    %%r15\n    2962:\tmov    %%r15,0x20(%%r12)\n    2966:\tret\n'
} >"$window_inline_both"
expect "check-disasm: a window that is only behind an inline lock has no unlocked path" 1 env UNLOCKED_PATH=1 sh "$here/check-disasm.sh" window "$window_inline_locked"
expect "check-disasm: an inline-locked copy and an unlocked copy are both found" 0 env UNLOCKED_PATH=1 sh "$here/check-disasm.sh" window "$window_inline_both"

# check-sources.sh --allow-instant: for the Rust probe alone, whose line carries the monotonic clock difference
instant="$tmp/instant.rs"
printf 'fn f() { let t = std::time::Instant::now(); let _ = t.elapsed(); }\n' >"$instant"
expect "check-sources rejects Instant::now by default" 1 sh "$here/check-sources.sh" "$instant"
expect "check-sources accepts Instant::now when it is allowed" 0 sh "$here/check-sources.sh" --allow-instant "$instant"
for token in SystemTime 'env::var' available_parallelism '/proc/self' HAIRSPRING_SEED; do
  bad="$tmp/bad3.rs"
  printf 'fn f() { use_it("%s"); let _ = Instant::now(); }\n' "$token" >"$bad"
  expect "check-sources --allow-instant still rejects $token" 1 sh "$here/check-sources.sh" --allow-instant "$bad"
done
expect "check-sources --allow-instant without a file is a usage error" 64 sh "$here/check-sources.sh" --allow-instant
expect "the real Rust probe is clean apart from Instant::now" 0 sh "$here/check-sources.sh" --allow-instant "$here"/../../../sdks/rust/fixtures/probe/src
expect "the real Rust probe is rejected when Instant::now is not allowed" 1 sh "$here/check-sources.sh" "$here"/../../../sdks/rust/fixtures/probe/src

# check-probe-writes.sh: B5 of the probe, on canned `strace -ff -s 4096 -e trace=write` text. No SDK, so no protocol lines.
pw_ok="$tmp/pw_ok.txt"
cat >"$pw_ok" <<'EOF'
160   write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148) = 148
EOF
pw_two="$tmp/pw_two.txt"
cat >"$pw_two" <<'EOF'
160   write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef ", 47) = 47
160   write(1, "arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 101) = 101
EOF
pw_none="$tmp/pw_none.txt"
printf '160   write(2, "something\\n", 10) = 10\n' >"$pw_none"
pw_other="$tmp/pw_other.txt"
cat >"$pw_other" <<'EOF'
160   write(1, "LANGAT v1 lang=rust mode=buggy final=00000000000000003990 expect=00000000000000004000 order=0123456789abcdef rand=0123456789abcdef\n", 128) = 128
EOF
pw_short="$tmp/pw_short.txt"
cat >"$pw_short" <<'EOF'
160   write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148) = 20
EOF
pw_event="$tmp/pw_event.txt"
cat >"$pw_event" <<'EOF'
160   write(200, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\"}\n", 38) = 38
160   write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148) = 148
EOF
pw_interleaved="$tmp/pw_interleaved.txt"
cat >"$pw_interleaved" <<'EOF'
161   write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148 <unfinished ...>
162   write(2, "x\n", 2) = 2
161   <... write resumed>) = 148
EOF
expect "check-probe-writes accepts one whole PROBE line on stdout" 0 sh "$here/check-probe-writes.sh" "$pw_ok"
expect "check-probe-writes joins an unfinished write to its resumed half" 0 sh "$here/check-probe-writes.sh" "$pw_interleaved"
expect "check-probe-writes rejects a line split across two writes" 1 sh "$here/check-probe-writes.sh" "$pw_two"
expect "check-probe-writes rejects a run with no write to stdout" 1 sh "$here/check-probe-writes.sh" "$pw_none"
expect "check-probe-writes rejects a stdout write that is not a PROBE line" 1 sh "$here/check-probe-writes.sh" "$pw_other"
expect "check-probe-writes rejects a short write" 1 sh "$here/check-probe-writes.sh" "$pw_short"
expect "check-probe-writes rejects a protocol line (the probe has no SDK)" 1 sh "$here/check-probe-writes.sh" "$pw_event"
expect "check-probe-writes without a file is a usage error" 64 sh "$here/check-probe-writes.sh"
# check-static.sh: the binary must be statically linked, read from the text of `file`
expect "check-static accepts a statically linked executable" 0 sh -c "echo 'x: ELF 64-bit LSB executable, x86-64, version 1 (SYSV), statically linked, stripped' | sh '$here/check-static.sh'"
expect "check-static accepts a static-pie executable" 0 sh -c "echo 'x: ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped' | sh '$here/check-static.sh'"
expect "check-static rejects a dynamically linked executable" 1 sh -c "echo 'x: ELF 64-bit LSB pie executable, x86-64, dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2' | sh '$here/check-static.sh'"
expect "check-static rejects empty input (no file command)" 1 sh -c "printf '' | sh '$here/check-static.sh'"
# check-instant-use.sh: exactly one Instant::now, no aliased import
inst_one="$tmp/inst_one.rs"
printf 'use std::time::Instant;\nfn f() { let _ = Instant::now(); }\n' >"$inst_one"
inst_two="$tmp/inst_two.rs"
printf 'use std::time::Instant;\nfn f() { let _ = Instant::now(); let _ = Instant::now(); }\n' >"$inst_two"
inst_alias="$tmp/inst_alias.rs"
printf 'use std::time::Instant as I;\nfn f() { let _ = Instant::now(); }\n' >"$inst_alias"
inst_none="$tmp/inst_none.rs"
printf 'fn f() {}\n' >"$inst_none"
expect "check-instant-use accepts exactly one Instant::now" 0 sh "$here/check-instant-use.sh" "$inst_one"
expect "check-instant-use rejects two reads" 1 sh "$here/check-instant-use.sh" "$inst_two"
expect "check-instant-use rejects an aliased import" 1 sh "$here/check-instant-use.sh" "$inst_alias"
expect "check-instant-use rejects a file with no read" 1 sh "$here/check-instant-use.sh" "$inst_none"
expect "check-instant-use without a file is a usage error" 64 sh "$here/check-instant-use.sh"
expect "the real probe has exactly one Instant::now" 0 sh "$here/check-instant-use.sh" "$here"/../../../sdks/rust/fixtures/probe/src/workload.rs
# Real `strace -ff -o` output: one file per thread, no pid column, and an unfinished write is resumed in the same file.
pw_real_main="$tmp/pw_real.100"
cat >"$pw_real_main" <<'EOF'
write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148 <unfinished ...>
<... write resumed>) = 148
+++ exited with 0 +++
EOF
pw_real_thread="$tmp/pw_real.101"
printf '+++ exited with 0 +++\n' >"$pw_real_thread"
pw_real_short="$tmp/pw_real_short.100"
cat >"$pw_real_short" <<'EOF'
write(1, "PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 148 <unfinished ...>
<... write resumed>) = 20
EOF
pw_real_two_files="$tmp/pw_real_two.101"
printf 'write(1, "PROBE v1 lang=rust x\\n", 20) = 20\n' >"$pw_real_two_files"
pw_writev="$tmp/pw_writev.txt"
cat >"$pw_writev" <<'EOF'
writev(1, [{iov_base="PROBE v1 lang=rust lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", iov_len=148}], 1) = 148
EOF
pw_writev_extra="$tmp/pw_writev_extra.txt"
{ cat "$pw_ok"; cat "$pw_writev"; } >"$pw_writev_extra"
expect "check-probe-writes accepts real per-thread files with an unfinished write resumed in the same file" 0 sh "$here/check-probe-writes.sh" "$pw_real_main" "$pw_real_thread"
expect "check-probe-writes rejects a short resumed write in a real per-thread file" 1 sh "$here/check-probe-writes.sh" "$pw_real_short" "$pw_real_thread"
expect "check-probe-writes counts stdout writes across the per-thread files" 1 sh "$here/check-probe-writes.sh" "$pw_real_main" "$pw_real_two_files"
expect "check-probe-writes rejects a stdout line written with writev" 1 sh "$here/check-probe-writes.sh" "$pw_writev"
expect "check-probe-writes rejects a writev to stdout beside a write" 1 sh "$here/check-probe-writes.sh" "$pw_writev_extra"

# check-probe-writes.sh --dup-of-stdout: the .NET runtime duplicates descriptor 1 for its console stream, so the line is
# written to another descriptor (real trace: fcntl(1, F_DUPFD_CLOEXEC, 0) = 24, then write(24, "PROBE v1 ...")). With the
# flag the line must be written once, whole, to a descriptor that a traced dup of descriptor 1 created; any write to
# descriptor 1 or 2 fails (the runtime writes nothing there); writes of other things (thread names, wake-up bytes) are
# ignored. A line on a duplicate of descriptor 2, or on a descriptor with no traced dup, is not accepted.
pw_dup_ok="$tmp/pw_dup_ok.txt"
cat >"$pw_dup_ok" <<'EOF'
fcntl(0, F_DUPFD_CLOEXEC, 0)            = 5
fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24
fcntl(2, F_DUPFD_CLOEXEC, 0)            = 27
write(13, "*", 1)                       = 1
write(14, ".NET Finalizer", 14)         = 14
write(24, "PROBE v1 lang=dotnet lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 149) = 149
write(4, "\1", 1)                       = 1
EOF
pw_dup_two="$tmp/pw_dup_two.txt"
cat >"$pw_dup_two" <<'EOF'
fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24
write(24, "PROBE v1 lang=dotnet lock_order=0123456789abcdef ", 49) = 49
write(24, "arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 100) = 100
EOF
pw_dup_short="$tmp/pw_dup_short.txt"
cat >"$pw_dup_short" <<'EOF'
fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24
write(24, "PROBE v1 lang=dotnet lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 149) = 20
EOF
pw_dup_none="$tmp/pw_dup_none.txt"
printf 'write(14, ".NET Finalizer", 14)         = 14\n' >"$pw_dup_none"
pw_dup_writev="$tmp/pw_dup_writev.txt"
cat >"$pw_dup_writev" <<'EOF'
fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24
writev(24, [{iov_base="PROBE v1 lang=dotnet lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", iov_len=149}], 1) = 149
EOF
pw_dup_event="$tmp/pw_dup_event.txt"
{ printf 'write(200, "HAIRSPRING_EVENT {\\"v\\":1}\\n", 24) = 24\n'; cat "$pw_dup_ok"; } >"$pw_dup_event"
# Writes that must not pass: something else on descriptor 1 beside the line on the duplicate, the line on descriptor 2, the
# line on a duplicate of descriptor 2, the line on a descriptor nobody duplicated from descriptor 1.
pw_dup_hello="$tmp/pw_dup_hello.txt"
{ printf 'write(1, "hello\\n", 6)                  = 6\n'; cat "$pw_dup_ok"; } >"$pw_dup_hello"
pw_dup_fd2="$tmp/pw_dup_fd2.txt"
printf 'write(2, "PROBE v1 lang=dotnet lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\\n", 149) = 149\n' >"$pw_dup_fd2"
pw_dup_of2="$tmp/pw_dup_of2.txt"
cat >"$pw_dup_of2" <<'EOF'
fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24
fcntl(2, F_DUPFD_CLOEXEC, 0)            = 27
write(27, "PROBE v1 lang=dotnet lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef\n", 149) = 149
EOF
pw_dup_nodup="$tmp/pw_dup_nodup.txt"
grep -v fcntl "$pw_dup_ok" >"$pw_dup_nodup"
pw_dup_later="$tmp/pw_dup_later.99"
printf 'fcntl(1, F_DUPFD_CLOEXEC, 0)            = 24\n' >"$pw_dup_later"
pw_dup_main="$tmp/pw_dup_main.98"
grep -v fcntl "$pw_dup_ok" >"$pw_dup_main"
expect "check-probe-writes --dup-of-stdout accepts the line on a duplicate of descriptor 1 among internal writes" 0 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_ok"
expect "check-probe-writes without --dup-of-stdout rejects a line that is not on descriptor 1" 1 sh "$here/check-probe-writes.sh" "$pw_dup_ok"
expect "check-probe-writes --dup-of-stdout finds the dup in another thread's file, whichever file comes first" 0 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_main" "$pw_dup_later"
expect "check-probe-writes --dup-of-stdout rejects other output on descriptor 1 beside the line" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_hello"
expect "check-probe-writes --dup-of-stdout rejects the line on descriptor 2" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_fd2"
expect "check-probe-writes --dup-of-stdout rejects the line on a duplicate of descriptor 2" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_of2"
expect "check-probe-writes --dup-of-stdout rejects a descriptor with no traced dup of descriptor 1" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_nodup"
expect "check-probe-writes --dup-of-stdout rejects a line written in two pieces" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_two"
expect "check-probe-writes --dup-of-stdout rejects a short write" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_short"
expect "check-probe-writes --dup-of-stdout rejects a run with no line" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_none"
expect "check-probe-writes --dup-of-stdout rejects a writev of the line" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_writev"
expect "check-probe-writes --dup-of-stdout rejects a protocol line" 1 sh "$here/check-probe-writes.sh" --dup-of-stdout "$pw_dup_event"
expect "check-probe-writes --dup-of-stdout without a file is a usage error" 64 sh "$here/check-probe-writes.sh" --dup-of-stdout

# check-sources-dotnet.sh: the .NET probe must not read the seed, /proc, the CPU count, the environment or the clock (it alone
# may read the stopwatch counter, once, and only in the file named by --allow-stopwatch-in).
dn_clean="$tmp/dn_clean.cs"
printf 'using System.Threading;\nclass C { void F() { new Thread(() => { }).Start(); } }\n' >"$dn_clean"
expect "check-sources-dotnet accepts a clean source" 0 sh "$here/check-sources-dotnet.sh" "$dn_clean"
for token in HAIRSPRING_SEED '/proc/self/stat' 'Environment.GetEnvironmentVariable("X")' 'Environment.GetEnvironmentVariables()' \
  'Environment.ExpandEnvironmentVariables("x")' 'Environment.ProcessorCount' 'Thread.GetCurrentProcessorId()' 'DateTime.Now' 'DateTime.UtcNow' \
  'DateTime.Today' 'DateTimeOffset.Now' 'DateTimeOffset.UtcNow' 'Environment.TickCount' 'Environment.TickCount64' \
  'TimeProvider.System.GetUtcNow()' 'Process.GetCurrentProcess()' 'Stopwatch.StartNew()' 'Stopwatch.GetTimestamp()' \
  'using static System.DateTime' 'using static System.DateTimeOffset' 'using static System.Environment' 'using static System.Diagnostics.Stopwatch' \
  'Now' 'UtcNow' 'Today' '"/sys/devices/system/cpu/online"' '"/sys/fs/cgroup/cpu.max"'; do
  dn_bad="$tmp/dn_bad.cs"
  printf 'class C { void F() { var x = %s; } }\n' "$token" >"$dn_bad"
  expect "check-sources-dotnet rejects $token" 1 sh "$here/check-sources-dotnet.sh" "$dn_bad"
done
dn_clock="$tmp/dn_clock.cs"
printf 'class C { long F() => System.Diagnostics.Stopwatch.GetTimestamp(); long G() => System.Diagnostics.Stopwatch.Frequency; }\n' >"$dn_clock"
expect "check-sources-dotnet accepts Stopwatch.GetTimestamp and Frequency when they are allowed" 0 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in dn_clock.cs "$dn_clock"
for token in 'Stopwatch.StartNew()' 'new Stopwatch()' 'Stopwatch.GetElapsedTime(1, 2)' 'Stopwatch.IsHighResolution'; do
  printf 'class C { void F() { var x = System.Diagnostics.%s; } }\n' "$token" >"$dn_bad"
  expect "check-sources-dotnet --allow-stopwatch-in still rejects $token" 1 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in dn_bad.cs "$dn_bad"
done
for token in HAIRSPRING_SEED '/proc/self' 'Environment.ProcessorCount' 'DateTime.UtcNow' 'Environment.GetEnvironmentVariable("X")'; do
  printf 'class C { void F() { var x = %s; var t = System.Diagnostics.Stopwatch.GetTimestamp(); } }\n' "$token" >"$dn_bad"
  expect "check-sources-dotnet --allow-stopwatch-in still rejects $token" 1 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in dn_bad.cs "$dn_bad"
done
expect "check-sources-dotnet without a file is a usage error" 64 sh "$here/check-sources-dotnet.sh"
expect "check-sources-dotnet --allow-stopwatch-in without a file is a usage error" 64 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in dn_clock.cs
expect "check-sources-dotnet --allow-stopwatch-in without a name is a usage error" 64 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in
expect "the allowance is for the named file only" 1 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in Other.cs "$dn_clock"
expect "check-sources-dotnet fails on a missing file" 1 sh "$here/check-sources-dotnet.sh" "$tmp/no-such-file.cs"
dn_dir="$tmp/dn_dir"
mkdir -p "$dn_dir/obj"
cp "$dn_clean" "$dn_dir/Clean.cs"
printf 'class D { long F() => System.Environment.TickCount64; }\n' >"$dn_dir/obj/Generated.cs"
expect "check-sources-dotnet searches a directory but not its build output" 0 sh "$here/check-sources-dotnet.sh" "$dn_dir"
printf 'class D { long F() => System.Environment.TickCount64; }\n' >"$dn_dir/Dirty.cs"
expect "check-sources-dotnet finds a forbidden read in a file of a directory" 1 sh "$here/check-sources-dotnet.sh" "$dn_dir"
dnroot="$here/../dotnet"
dn_two="$tmp/dn_two"
mkdir -p "$dn_two"
cp "$dnroot/src/Clock.cs" "$dn_two/Clock.cs"
expect "a directory with the one clock file passes when the allowance names it" 0 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in Clock.cs "$dn_two"
printf 'class Second { long F() => System.Diagnostics.Stopwatch.GetTimestamp(); }\n' >"$dn_two/Second.cs"
expect "a second clock read in another file is rejected" 1 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in Clock.cs "$dn_two"
expect "the real .NET probe is clean apart from the stopwatch in Clock.cs" 0 sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in Clock.cs "$dnroot/src"
expect "the real .NET probe is rejected when the stopwatch is not allowed" 1 sh "$here/check-sources-dotnet.sh" "$dnroot/src"

# check-stopwatch-use.sh: exactly one Stopwatch.GetTimestamp, no aliased import of Stopwatch
sw_one="$tmp/sw_one.cs"
printf 'using System.Diagnostics;\nclass C { long F() => Stopwatch.GetTimestamp(); }\n' >"$sw_one"
sw_two="$tmp/sw_two.cs"
printf 'using System.Diagnostics;\nclass C { long F() => Stopwatch.GetTimestamp() + Stopwatch.GetTimestamp(); }\n' >"$sw_two"
sw_alias="$tmp/sw_alias.cs"
printf 'using S = System.Diagnostics.Stopwatch;\nclass C { long F() => System.Diagnostics.Stopwatch.GetTimestamp(); }\n' >"$sw_alias"
sw_static="$tmp/sw_static.cs"
printf 'using static System.Diagnostics.Stopwatch;\nclass C { long F() => System.Diagnostics.Stopwatch.GetTimestamp(); }\n' >"$sw_static"
sw_none="$tmp/sw_none.cs"
printf 'class C { }\n' >"$sw_none"
expect "check-stopwatch-use accepts exactly one Stopwatch.GetTimestamp" 0 sh "$here/check-stopwatch-use.sh" "$sw_one"
expect "check-stopwatch-use rejects two reads" 1 sh "$here/check-stopwatch-use.sh" "$sw_two"
expect "check-stopwatch-use rejects an aliased import" 1 sh "$here/check-stopwatch-use.sh" "$sw_alias"
expect "check-stopwatch-use rejects a static import" 1 sh "$here/check-stopwatch-use.sh" "$sw_static"
expect "check-stopwatch-use rejects a file with no read" 1 sh "$here/check-stopwatch-use.sh" "$sw_none"
expect "check-stopwatch-use without a file is a usage error" 64 sh "$here/check-stopwatch-use.sh"
expect "the real .NET probe has exactly one Stopwatch.GetTimestamp" 0 sh "$here/check-stopwatch-use.sh" "$dnroot/src/Clock.cs"
# No other source file of the probe may read the clock at all.
expect "no other source of the .NET probe mentions Stopwatch" 1 sh -c "grep -l Stopwatch '$dnroot'/src/*.cs | grep -v 'src/Clock.cs'"

# extract-runtime.sh: the pinned runtime tarball is checked against its sha512 before it is extracted
rt_src="$tmp/rt_src"
mkdir -p "$rt_src/shared"
printf '#!/bin/sh\n' >"$rt_src/dotnet"
printf 'x\n' >"$rt_src/shared/file"
tar -czf "$tmp/rt.tar.gz" -C "$rt_src" .
if command -v sha512sum >/dev/null 2>&1; then rt_sha=$(sha512sum "$tmp/rt.tar.gz" | cut -d' ' -f1); else rt_sha=$(shasum -a 512 "$tmp/rt.tar.gz" | cut -d' ' -f1); fi
expect "extract-runtime extracts a tarball whose sha512 matches" 0 sh "$here/extract-runtime.sh" "$tmp/rt.tar.gz" "$rt_sha" "$tmp/rt_out"
expect "the extracted runtime holds the dotnet file" 0 test -e "$tmp/rt_out/dotnet"
expect "extract-runtime rejects a wrong sha512" 1 sh "$here/extract-runtime.sh" "$tmp/rt.tar.gz" "$(printf '%0128d' 0)" "$tmp/rt_out2"
expect "nothing is extracted when the sha512 is wrong" 1 test -e "$tmp/rt_out2"
expect "extract-runtime rejects a sha512 of the wrong length" 1 sh "$here/extract-runtime.sh" "$tmp/rt.tar.gz" abc "$tmp/rt_out3"
expect "extract-runtime fails on a missing tarball" 1 sh "$here/extract-runtime.sh" "$tmp/no-such.tar.gz" "$rt_sha" "$tmp/rt_out4"
expect "extract-runtime refuses an existing destination" 1 sh "$here/extract-runtime.sh" "$tmp/rt.tar.gz" "$rt_sha" "$tmp/rt_out"
expect "extract-runtime without arguments is a usage error" 64 sh "$here/extract-runtime.sh"

# profile-env.sh: the probe's environment profiles, read from profiles.manifest
pe_manifest="$tmp/pe.manifest"
printf '# comment\n\na DOTNET_ROOT=@DOTNET_ROOT@\nb DOTNET_X=1\nb DOTNET_Y=2\n' >"$pe_manifest"
expect "profile-env prints arm a" 0 sh -c "[ \"\$(sh '$here/../dotnet/profile-env.sh' a /rt '$pe_manifest')\" = 'DOTNET_ROOT=/rt' ]"
expect "profile-env prints arm a then the b lines for arm b" 0 sh -c "[ \"\$(sh '$here/../dotnet/profile-env.sh' b /rt '$pe_manifest')\" = 'DOTNET_ROOT=/rt
DOTNET_X=1
DOTNET_Y=2' ]"
printf 'a DOTNET_ROOT=@DOTNET_ROOT@\nb DOTNET_X = 1\n' >"$tmp/pe_bad.manifest"
expect "profile-env rejects a malformed line even for arm a" 1 sh "$here/../dotnet/profile-env.sh" a /rt "$tmp/pe_bad.manifest"
printf 'c DOTNET_X=1\n' >"$tmp/pe_arm.manifest"
expect "profile-env rejects a line for an unknown arm" 1 sh "$here/../dotnet/profile-env.sh" b /rt "$tmp/pe_arm.manifest"
expect "profile-env fails on a missing manifest" 1 sh "$here/../dotnet/profile-env.sh" a /rt "$tmp/no-such.manifest"
expect "profile-env with one argument is a usage error" 64 sh "$here/../dotnet/profile-env.sh" a
expect "profile-env with an unknown arm is a usage error" 64 sh "$here/../dotnet/profile-env.sh" z /rt

# check-sources-java.sh: the Java probe must not read the scheduler seed, /proc, the environment, the CPU count, the process
# table or the clock (System.nanoTime twice, the start and the end of the run, only when allowed).
jclean="$tmp/Clean.java"
printf 'class Clean { long f() { return 1L; } }\n' >"$jclean"
expect "check-sources-java accepts a clean source" 0 sh "$here/check-sources-java.sh" "$jclean"
for token in HAIRSPRING_SEED '/proc/self/stat' 'System.getenv("X")' 'System.getenv()' 'Runtime.getRuntime().availableProcessors()' \
  'ProcessHandle.current()' 'new ProcessBuilder("ls")' 'System.currentTimeMillis()' 'Instant.now()' 'LocalDateTime.now()' \
  'Clock.systemUTC()' 'new java.util.Date()' 'ManagementFactory.getRuntimeMXBean()' 'System.nanoTime()'; do
  jbad="$tmp/Bad.java"
  printf 'class Bad { Object f() { return %s; } }\n' "$token" >"$jbad"
  expect "check-sources-java rejects $token" 1 sh "$here/check-sources-java.sh" "$jbad"
done
jtwo="$tmp/Two.java"
printf 'class Two { long f() { long a = System.nanoTime(); long b = System.nanoTime(); return b - a; } }\n' >"$jtwo"
jone="$tmp/One.java"
printf 'class One { long f() { return System.nanoTime(); } }\n' >"$jone"
jthree="$tmp/Three.java"
printf 'class Three { long f() { return System.nanoTime() + System.nanoTime() + System.nanoTime(); } }\n' >"$jthree"
jalias="$tmp/Alias.java"
printf 'import static java.lang.System.nanoTime;\nclass Alias { long f() { long a = nanoTime(); return System.nanoTime() - a; } }\n' >"$jalias"
expect "check-sources-java accepts exactly two System.nanoTime reads when they are allowed" 0 sh "$here/check-sources-java.sh" --allow-nano-time "$jtwo"
expect "check-sources-java rejects one read when two are required" 1 sh "$here/check-sources-java.sh" --allow-nano-time "$jone"
expect "check-sources-java rejects three reads" 1 sh "$here/check-sources-java.sh" --allow-nano-time "$jthree"
expect "check-sources-java rejects a static import that hides a read" 1 sh "$here/check-sources-java.sh" --allow-nano-time "$jalias"
jboth="$tmp/Both.java"
printf 'class Both { long f() { System.currentTimeMillis(); long a = System.nanoTime(); return System.nanoTime() - a; } }\n' >"$jboth"
expect "check-sources-java --allow-nano-time still rejects the wall clock" 1 sh "$here/check-sources-java.sh" --allow-nano-time "$jboth"
expect "check-sources-java without a file is a usage error" 64 sh "$here/check-sources-java.sh"
expect "check-sources-java with a missing file fails" 1 sh "$here/check-sources-java.sh" "$tmp/NoSuch.java"
expect "check-sources-java searches a directory" 0 sh "$here/check-sources-java.sh" --allow-nano-time "$here"/../java/src
expect "the real Java probe is rejected when System.nanoTime is not allowed" 1 sh "$here/check-sources-java.sh" "$here"/../java/src

# check-java-profiles.sh: every flag profile is a valid set of JVM options, and a run under it prints the one probe line and
# nothing else (a JVM warning goes to stdout by default, which would break the one-line contract). A fake java stands in.
fakejava="$tmp/fakejava"
cat >"$fakejava" <<'EOF'
#!/bin/sh
case "$*" in
  *-version*)
    [ "${FAKE:-}" = badflag ] && { echo "Unrecognized VM option 'Nope'" >&2; exit 1; }
    [ "${FAKE:-}" = versionwarn ] && echo "OpenJDK 64-Bit Server VM warning: Option X was deprecated" >&2
    echo 'openjdk version "21.0.12.1" 2026-07-21 LTS' >&2
    exit 0
    ;;
esac
[ "${FAKE:-}" = warn ] && echo "OpenJDK 64-Bit Server VM warning: something"
[ "${FAKE:-}" = noisy ] && echo "noise" >&2
[ "${FAKE:-}" = fail ] && exit 1
[ "${FAKE:-}" = twolines ] && echo "second line"
fakeline='PROBE v1 lang=java lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef'
printf '%s\n' "$fakeline"
[ "${FAKE:-}" = twice ] && printf '%s\n' "$fakeline"
[ "${FAKE:-}" = junk ] && printf 'junk'
exit 0
EOF
chmod +x "$fakejava"
profiles="$tmp/profiles.tsv"
printf '# comment\na\t-Xint -XX:+UseSerialGC\nb\t-XX:TieredStopAtLevel=1\n' >"$profiles"
jarfile="$tmp/fake.jar"
: >"$jarfile"
cjp() { # cjp <name> <wanted exit> <FAKE value> [profiles file]
  expect "check-java-profiles $1" "$2" env FAKE="$3" JAVA="$fakejava" sh "$here/check-java-profiles.sh" "${4:-$profiles}" "$jarfile"
}
cjp "accepts profiles that a JVM accepts and that print one clean line" 0 ""
cjp "rejects a flag the JVM refuses" 1 badflag
cjp "rejects a JVM warning printed by -version" 1 versionwarn
cjp "rejects a JVM warning on stdout in front of the line" 1 warn
cjp "rejects output on stderr" 1 noisy
cjp "rejects a failing probe" 1 fail
cjp "rejects a second stdout line" 1 twolines
printf '# nothing\n' >"$tmp/empty.tsv"
cjp "rejects a profiles file with no profile" 1 "" "$tmp/empty.tsv"
cjp "rejects a missing profiles file" 1 "" "$tmp/no-such.tsv"
expect "check-java-profiles without arguments is a usage error" 64 sh "$here/check-java-profiles.sh"
expect "check-java-profiles prints each arm and its flags" 0 sh -c "FAKE= JAVA='$fakejava' sh '$here/check-java-profiles.sh' '$profiles' '$jarfile' | grep -q 'arm b: -XX:TieredStopAtLevel=1'"
# The real manifest of flags: exactly the two profiles of the experiment, no other flag.
expect "the real profiles.tsv names arm a with the base flags" 0 grep -Fxq "$(printf 'a\t-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off')" "$here/../java/profiles.tsv"
expect "the real profiles.tsv names arm b with the JIT-limited flags" 0 grep -Fxq "$(printf 'b\t-XX:TieredStopAtLevel=1 -XX:-BackgroundCompilation -XX:CICompilerCount=1 -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off')" "$here/../java/profiles.tsv"
expect "the real profiles.tsv has two profiles and nothing else but comments" 0 sh -c "test \$(grep -vc '^#' '$here/../java/profiles.tsv') -eq 2"

# check-java-profiles.sh --exact: the profiles file must be exactly arms a and b with the experiment's flags.
cjx() { # cjx <name> <wanted exit> <profiles file>
  expect "check-java-profiles --exact $1" "$2" env FAKE= JAVA="$fakejava" sh "$here/check-java-profiles.sh" --exact "$3" "$jarfile"
}
cjx "accepts the real profiles" 0 "$here/../java/profiles.tsv"
cjx "rejects profiles with other flags" 1 "$profiles"
{ cat "$here/../java/profiles.tsv"; printf 'c\t-Xint\n'; } >"$tmp/three.tsv"
cjx "rejects a third arm" 1 "$tmp/three.tsv"
sed 's/-Xshare:off/-Xshare:auto/' "$here/../java/profiles.tsv" >"$tmp/changed.tsv"
cjx "rejects a changed flag" 1 "$tmp/changed.tsv"
grep -v '^b' "$here/../java/profiles.tsv" >"$tmp/onlya.tsv"
cjx "rejects a missing arm" 1 "$tmp/onlya.tsv"
# Trailing junk after the line: the line must be the whole of stdout.
cjp "rejects junk after the line (no newline at its end)" 1 junk
cjp "rejects a stdout that is the line twice" 1 twice

# check-probe-line.sh: one line in the six-field form and nothing else, byte for byte.
goodline='PROBE v1 lang=java lock_order=0123456789abcdef arrival=0123456789abcdef rand=0123456789abcdef vtime_ns=00000000000005000000 extra=0123456789abcdef'
printf '%s\n' "$goodline" >"$tmp/pl_good.txt"
printf '%s\njunk' "$goodline" >"$tmp/pl_junk.txt"
printf '%s\n\n' "$goodline" >"$tmp/pl_blank.txt"
printf '%s' "$goodline" >"$tmp/pl_nonl.txt"
printf '%s\n%s\n' "$goodline" "$goodline" >"$tmp/pl_two.txt"
printf '%s \n' "$goodline" >"$tmp/pl_trail.txt"
: >"$tmp/pl_empty.txt"
expect "check-probe-line accepts the line and its newline" 0 sh "$here/check-probe-line.sh" "$tmp/pl_good.txt"
expect "check-probe-line rejects LINE then junk without a newline" 1 sh "$here/check-probe-line.sh" "$tmp/pl_junk.txt"
expect "check-probe-line rejects a blank line after the line" 1 sh "$here/check-probe-line.sh" "$tmp/pl_blank.txt"
expect "check-probe-line rejects a line with no newline" 1 sh "$here/check-probe-line.sh" "$tmp/pl_nonl.txt"
expect "check-probe-line rejects two lines" 1 sh "$here/check-probe-line.sh" "$tmp/pl_two.txt"
expect "check-probe-line rejects trailing space" 1 sh "$here/check-probe-line.sh" "$tmp/pl_trail.txt"
expect "check-probe-line rejects an empty file" 1 sh "$here/check-probe-line.sh" "$tmp/pl_empty.txt"
expect "check-probe-line rejects a missing file" 1 sh "$here/check-probe-line.sh" "$tmp/pl_missing.txt"
expect "check-probe-line without a file is a usage error" 64 sh "$here/check-probe-line.sh"

# check-jdk-version.sh: the JDK must report exactly the pinned build, not a later one that starts with it.
jv() { # jv <name> <wanted exit> <text>
  expect "check-jdk-version $1" "$2" sh -c "printf '%s\n' '$3' | sh '$here/check-jdk-version.sh' Temurin-21.0.12.1+1"
}
jv "accepts the pinned build" 0 'OpenJDK Runtime Environment Temurin-21.0.12.1+1 (build 21.0.12.1+1-LTS)'
jv "accepts the pinned build at the end of a line" 0 'Runtime Temurin-21.0.12.1+1'
jv "rejects +10" 1 'OpenJDK Runtime Environment Temurin-21.0.12.1+10 (build 21.0.12.1+10-LTS)'
jv "rejects +19" 1 'OpenJDK Runtime Environment Temurin-21.0.12.1+19 (build 21.0.12.1+19-LTS)'
jv "rejects another patch release" 1 'OpenJDK Runtime Environment Temurin-21.0.13+9 (build 21.0.13+9-LTS)'
jv "rejects a dot standing for another character" 1 'OpenJDK Runtime Environment Temurin-21x0.12.1+1 (build 21)'
expect "check-jdk-version without a pin is a usage error" 64 sh "$here/check-jdk-version.sh"

# check-sources-java.sh: more clocks and random sources, system properties, /sys and reflection.
for token in 'Calendar.getInstance()' 'OffsetDateTime.now()' 'InstantSource.system()' 'Year.now()' 'YearMonth.now()' 'MonthDay.now()' \
  'System.getProperty("os.name")' 'System.getProperties()' '"/sys/devices/system/cpu"' 'Class.forName("x")' 'x.getDeclaredMethod("m")' \
  'f.setAccessible(true)' 'ThreadLocalRandom.current()' 'Math.random()' 'new SecureRandom()' 'UUID.randomUUID()' \
  'new SplittableRandom()' 'RandomGenerator.getDefault()' 'RandomGeneratorFactory.of("L64X128MixRandom")' \
  'Runtime.getRuntime().exec("ls")' 'new ProcessBuilder("ls")'; do
  jbad="$tmp/Bad2.java"
  printf 'class Bad2 { Object f() { return %s; } }\n' "$token" >"$jbad"
  expect "check-sources-java rejects $token" 1 sh "$here/check-sources-java.sh" "$jbad"
done

libcase "a record without an alignment checklist does not claim a count of ticked lines" 0 'record_title=t; record_generated_by=g; artifact_name=p; artifact_sha=aa; record_no_checklist=1; finish_record >/dev/null; ! grep -q "Checklist lines ticked" "$out"'
libcase "a record with an alignment checklist states how many lines are ticked" 0 'record_title=t; record_generated_by=g; artifact_name=p; artifact_sha=aa; finish_record >/dev/null; grep -q "^Checklist lines ticked: 0 of 11." "$out"'

libcase "a staged package is identified by its tree hash in the artifact line" 0 'record_title=t; record_generated_by=g; artifact_name=at-py; artifact_sha_label="tree hash"; artifact_sha=bb; finish_record >/dev/null; grep -q "^Artifact: at-py, tree hash bb" "$out"'
libcase "the finished record starts with the title, then the producer fields and the verdicts, then the steps" 0 'step B1 t true; step B2 t false; record_title="BUILD-RECORD: at-x"; record_generated_by="Generated by x."; artifact_name=at-x; artifact_sha=aa; finish_record >/dev/null; [ "$(head -1 "$out")" = "# BUILD-RECORD: at-x" ] && grep -q "^Verdicts: B1 PASS, B2 FAIL" "$out" && grep -q "^Artifact: at-x, sha256 aa" "$out" && grep -q "the box gets exactly those bytes" "$out" && [ "$(grep -n "^Verdicts" "$out" | cut -d: -f1)" -lt "$(grep -n "^### B1" "$out" | cut -d: -f1)" ]'

# The Java fixture: its constants and start barrier are read from its source, its sources read nothing forbidden, and
# check-java-arms.sh judges the two JVM flag profiles (here with a fake java, so no JDK is needed).
java_fixture="$fixtures_dir/java"
expect "constants evidence passes for the Java fixture" 0 sh "$evidence" constants java "$java_fixture/at/src/langat/Contract.java"
sed 's/WINDOW = 1000;/WINDOW = 2000;/' "$java_fixture/at/src/langat/Contract.java" >"$tmp/Contract_w2000.java"
expect "a Java window of 2000 is rejected" 1 sh "$evidence" constants java "$tmp/Contract_w2000.java"
sed 's/WINDOW_WIDE = 10000;/WINDOW_WIDE = 20000;/' "$java_fixture/at/src/langat/Contract.java" >"$tmp/Contract_wide.java"
expect "a Java wide window of 20000 is rejected" 1 sh "$evidence" constants java "$tmp/Contract_wide.java"
sed 's/ITERATIONS = 1000;/ITERATIONS = 999;/' "$java_fixture/at/src/langat/Contract.java" >"$tmp/Contract_k999.java"
expect "a Java iteration count of 999 is rejected" 1 sh "$evidence" constants java "$tmp/Contract_k999.java"
expect "the real Java fixture has a start barrier" 0 sh "$evidence" barrier "$java_fixture/at/src/langat/Race.java"
expect "the Java fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait java "$java_fixture/at/src/langat/Race.java" 'barrier.await()')"
barrier_java_assert="$tmp/BarrierAssert.java"
printf 'void work() {\n  started.incrementAndGet();\n  run();\n}\nvoid main() {\n  Hairspring.sometimes(started.get() == THREADS, "java.all_started", "x");\n}\n' >"$barrier_java_assert"
expect "barrier evidence rejects a Java increment with no wait" 1 sh "$evidence" barrier "$barrier_java_assert"
expect "the real Java fixture reads nothing forbidden" 0 sh "$here/check-sources-java.sh" "$java_fixture/at/src"
bad_java="$tmp/Bad.java"
printf 'class Bad { long t = System.nanoTime(); }\n' >"$bad_java"
expect "a Java fixture that reads the clock is rejected" 1 sh "$here/check-sources-java.sh" "$bad_java"

arms_java="$tmp/arms-java"
cat >"$arms_java" <<'EOF'
#!/bin/sh
# FAKE_JAVA selects how it misbehaves; FAKE_JAVA_LOG, if set, receives the arguments of every call.
[ -z "${FAKE_JAVA_LOG:-}" ] || echo "$*" >>"$FAKE_JAVA_LOG"
for a in "$@"; do
  if [ "$a" = -version ]; then
    echo 'openjdk version "21.0.5" 2024-10-15 LTS' >&2
    [ "${FAKE_JAVA:-}" = warn ] && echo 'OpenJDK 64-Bit Server VM warning: ignoring option' >&2
    [ "${FAKE_JAVA:-}" = refuse ] && exit 1
    exit 0
  fi
done
for a in "$@"; do mode=$a; done
final=4000
[ "${FAKE_JAVA:-}" = fixedloses ] && [ "$mode" = fixed ] && final=3990
[ "${FAKE_JAVA:-}" = buggyfires ] && [ "$mode" = buggy ] && final=3990
printf 'LANGAT v1 lang=java mode=%s final=%020d expect=%020d order=0123456789abcdef rand=0123456789abcdef\n' "$mode" "$final" 4000
[ "${FAKE_JAVA:-}" = extraline ] && echo "done"
echo 'HAIRSPRING_EVENT {"v":1,"kind":"end"}' >&2
[ "${FAKE_JAVA:-}" = junk ] && echo "Picked up JAVA_TOOL_OPTIONS: -Xmx1g" >&2
[ "${FAKE_JAVA:-}" = badexit ] && exit 1
exit 0
EOF
chmod +x "$arms_java"
profiles_tsv="$java_fixture/profiles.tsv"
expect "java arms: both profiles pass with a conforming java" 0 env JAVA="$arms_java" sh "$here/check-java-arms.sh" --exact "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a java that refuses the flags is rejected" 1 env FAKE_JAVA=refuse JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a JVM warning is rejected" 1 env FAKE_JAVA=warn JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a locked mode that loses an update is rejected" 1 env FAKE_JAVA=fixedloses JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: an extra stdout line is rejected" 1 env FAKE_JAVA=extraline JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a JVM message on stderr is rejected" 1 env FAKE_JAVA=junk JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a non-zero exit is rejected" 1 env FAKE_JAVA=badexit JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: a buggy mode that never fires is information, not a failure" 0 env JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "java arms: it says how often buggy fired" 0 sh -c "env FAKE_JAVA=buggyfires JAVA='$arms_java' ARM_RUNS=3 sh '$here/check-java-arms.sh' '$profiles_tsv' '$tmp/fake.jar' | grep -q 'arm a, buggy lost an update in 3 of 3 runs'"
java_log="$tmp/java.log"
: >"$java_log"
env FAKE_JAVA_LOG="$java_log" JAVA="$arms_java" sh "$here/check-java-arms.sh" "$profiles_tsv" "$tmp/fake.jar" >/dev/null 2>&1
expect "java arms: arm a runs the program under -Xint" 0 sh -c "grep -q -e '-Xint .* -cp $tmp/fake.jar langat.Main fixed' '$java_log'"
expect "java arms: arm b runs the program under the C1-only flags" 0 sh -c "grep -q -e '-XX:TieredStopAtLevel=1 .* -cp $tmp/fake.jar langat.Main buggy' '$java_log'"
printf 'a\t-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off\n' >"$tmp/only_a.tsv"
expect "java arms: --exact rejects a profile file with only arm a" 1 env JAVA="$arms_java" sh "$here/check-java-arms.sh" --exact "$tmp/only_a.tsv" "$tmp/fake.jar"
printf 'a\t-Xint -XX:+UseSerialGC\nb\t-XX:TieredStopAtLevel=1\n' >"$tmp/other_flags.tsv"
expect "java arms: --exact rejects other flags" 1 env JAVA="$arms_java" sh "$here/check-java-arms.sh" --exact "$tmp/other_flags.tsv" "$tmp/fake.jar"
printf '# only a comment\n' >"$tmp/no_profiles.tsv"
expect "java arms: a file with no profile fails" 1 env JAVA="$arms_java" sh "$here/check-java-arms.sh" "$tmp/no_profiles.tsv" "$tmp/fake.jar"
expect "java arms: an unreadable profile file fails" 1 env JAVA="$arms_java" sh "$here/check-java-arms.sh" "$tmp/none.tsv" "$tmp/fake.jar"
expect "java arms: without arguments is a usage error" 64 sh "$here/check-java-arms.sh"

# check-java-compiled.sh: -XX:+PrintCompilation must never name the two worker loops, under the default flags and each profile.
compiled_java="$tmp/compiled-java"
cat >"$compiled_java" <<'EOF2'
#!/bin/sh
# FAKE_JAVA: workerloop (a worker loop is compiled), otherrace (another Race method is compiled), spinx (a method whose name
# starts with spin, langat.Race::spinX, is compiled), nospin (compile lines but
# never spin), spinonlywide (spin is compiled in wide, and no other line in any mode), nocompiles (PrintCompilation prints
# nothing), exit1 (the program fails).
interp=0
big=0
prev=
for a in "$@"; do
  [ "$a" = -Xint ] && interp=1
  [ "$prev" = langat.Main ] && mode=$a
  case "$a" in --k=*) big=1 ;; esac
  prev=$a
done
[ "${FAKE_JAVA:-}" = exit1 ] && exit 1
# kloop: the locked worker loop is compiled only when --k is given (the large-K case).
[ "${FAKE_JAVA:-}" = kloop ] && [ "$big" -eq 1 ] && [ "$interp" -eq 0 ] && echo '    900   99 %     3       langat.Race::lockedIterations @ 21 (66 bytes)'

if [ "$interp" -eq 0 ] && [ "${FAKE_JAVA:-}" = spinonlywide ]; then
  [ "$mode" = wide ] && echo '    130   40 %     3       langat.Race::spin @ 8 (30 bytes)'
elif [ "$interp" -eq 0 ] && [ "${FAKE_JAVA:-}" != nocompiles ]; then
  echo '     42   17       3       java.lang.String::hashCode (55 bytes)'
  [ "$mode" = wide ] && [ "${FAKE_JAVA:-}" != nospin ] && echo '    130   40 %     3       langat.Race::spin @ 8 (30 bytes)'
fi
[ "${FAKE_JAVA:-}" = otherrace ] && [ "$interp" -eq 0 ] && echo '    500   60       3       langat.Race::joinAll (40 bytes)'
[ "${FAKE_JAVA:-}" = spinx ] && [ "$interp" -eq 0 ] && echo '    500   62       3       langat.Race::spinX (10 bytes)'
[ "${FAKE_JAVA:-}" = otherrace2 ] && [ "$interp" -eq 0 ] && echo '    500   61       3       langat.Race::lambda$execute$0 (10 bytes)'
[ "${FAKE_JAVA:-}" = workerloop ] && [ "$mode" = fixed ] && [ "$interp" -eq 0 ] && echo '    900   99 %     3       langat.Race::lockedIterations @ 21 (66 bytes)'
[ "${FAKE_JAVA:-}" = workerracy ] && [ "$mode" = buggy ] && [ "$interp" -eq 0 ] && echo '    900   98       3       langat.Race::racyIterations (66 bytes)'
printf 'LANGAT v1 lang=java mode=%s final=%020d expect=%020d order=0123456789abcdef rand=0123456789abcdef\n' "$mode" 4000 4000
exit 0
EOF2
chmod +x "$compiled_java"
cjc() { # cjc <name> <wanted exit> <FAKE_JAVA value> [profiles file]
  expect "check-java-compiled $1" "$2" env FAKE_JAVA="$3" JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "${4:-$profiles_tsv}" "$tmp/fake.jar"
}
cjc "accepts runs in which neither worker loop is compiled" 0 ""
cjc "rejects a compiled locked worker loop (an OSR compile)" 1 workerloop
cjc "rejects a compiled unlocked worker loop" 1 workerracy
cjc "rejects any other compiled Race method" 1 otherrace
cjc "rejects a compiled Race lambda" 1 otherrace2
cjc "rejects a compiled method whose name only starts with spin (langat.Race::spinX)" 1 spinx
cjc "rejects a log with compile lines but no spin in wide (the spin control)" 1 nospin
cjc "rejects a log with no compile line in buggy although wide shows spin (the compile-line control)" 1 spinonlywide
cjc "rejects a PrintCompilation that prints nothing (the check could not see a compile)" 1 nocompiles
cjc "rejects a program that fails" 1 exit1
cjc "rejects a missing profiles file" 1 "" "$tmp/none.tsv"
expect "check-java-compiled accepts clean runs at the default K and at max-k" 0 env FAKE_JAVA= JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" 5000
expect "check-java-compiled without max-k does not see a loop compiled only at a large K" 0 env FAKE_JAVA=kloop JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar"
expect "check-java-compiled with max-k rejects a loop compiled only at a large K" 1 env FAKE_JAVA=kloop JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" 5000
expect "check-java-compiled rejects an empty max-k (present, not absent)" 64 env JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" ""
expect "check-java-compiled rejects a zero max-k" 64 env JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" 0
expect "check-java-compiled rejects a four-argument call" 64 env JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" 5000 extra
cap=$(sed -n 's/.*MAX_ITERATIONS = \([0-9]*\);.*/\1/p' "$here/../java/at/src/langat/Contract.java")
expect "the Java Makefile's at-compiled passes the cap read from Contract.java" 0 sh -c "make -n -C '$here/../java' at-compiled | grep -q 'check-java-compiled.sh profiles.tsv .*lang-at.jar $cap\$'"
expect "the Java Makefile reads a cap" 0 sh -c "test -n '$cap'"
expect "the Java Makefile's at-compiled fails when the cap is empty" 2 make -C "$here/../java" at-compiled MAX_K=
expect "the Java Makefile's at-compiled fails when the cap is not a number" 2 make -C "$here/../java" at-compiled MAX_K=lots
expect "the Java Makefile's cap check passes with the real cap" 0 make -C "$here/../java" at-max-k-check
expect "check-java-compiled rejects a max-k that is not a number" 64 env JAVA="$compiled_java" sh "$here/check-java-compiled.sh" "$profiles_tsv" "$tmp/fake.jar" lots
expect "check-java-compiled runs every arm again at max-k" 0 sh -c "env JAVA='$compiled_java' sh '$here/check-java-compiled.sh' '$profiles_tsv' '$tmp/fake.jar' 5000 | grep -c '^arm ' | grep -qx 6"
expect "check-java-compiled without arguments is a usage error" 64 sh "$here/check-java-compiled.sh"
expect "check-java-compiled names the default flags and each profile" 0 sh -c "env JAVA='$compiled_java' sh '$here/check-java-compiled.sh' '$profiles_tsv' '$tmp/fake.jar' | grep -c '^arm ' | grep -qx 3"

# The extra profile a-prime: profile a with -Xshare:off replaced by -Xshare:auto. It lives in its own file so that
# profiles.tsv (pinned to arms a and b by --exact, the build record and the tests above) is unchanged.
extra_tsv="$here/../java/profiles-extra.tsv"
expect "profiles-extra.tsv names a-prime: profile a with -Xshare:auto in place of -Xshare:off" 0 sh -c "test \"\$(grep -v '^#' '$extra_tsv' | grep -v '^\$')\" = \"\$(grep '^a	' '$profiles_tsv' | sed 's/^a	/a-prime	/; s/-Xshare:off/-Xshare:auto/')\""
expect "profiles-extra.tsv holds a-prime and nothing else but comments" 0 sh -c "test \$(grep -vc '^#' '$extra_tsv') -eq 1"
expect "profiles-extra.tsv's flags are exactly the five of the acceptance profile" 0 grep -Fxq "$(printf 'a-prime\t-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:auto')" "$extra_tsv"
expect "profiles.tsv still holds exactly arms a and b" 0 sh -c "test \$(grep -vc '^#' '$profiles_tsv') -eq 2 && ! grep -q a-prime '$profiles_tsv'"
expect "check-java-arms accepts profiles-extra.tsv with a conforming java" 0 env FAKE_JAVA= JAVA="$arms_java" sh "$here/check-java-arms.sh" "$extra_tsv" "$tmp/fake.jar"
expect "check-java-compiled accepts profiles-extra.tsv (default flags, a-prime, each at max-k)" 0 sh -c "env JAVA='$compiled_java' sh '$here/check-java-compiled.sh' '$extra_tsv' '$tmp/fake.jar' 5000 | grep -c '^arm ' | grep -qx 4"
jmake="make -n -C $here/../java"
expect "the Java Makefile's at-compiled also checks profiles-extra.tsv" 0 sh -c "$jmake at-compiled | grep -q 'check-java-compiled.sh profiles-extra.tsv .*lang-at.jar $cap\$'"
expect "the Java Makefile's at-compiled still checks profiles.tsv with the cap" 0 sh -c "$jmake at-compiled | grep -q 'check-java-compiled.sh profiles.tsv .*lang-at.jar $cap\$'"
expect "the Java Makefile's at-arms also checks profiles-extra.tsv" 0 sh -c "$jmake at-arms | grep -q 'check-java-arms.sh profiles-extra.tsv'"
expect "the Java Makefile's at-arms still checks profiles.tsv" 0 sh -c "$jmake at-arms | grep -q 'check-java-arms.sh profiles.tsv'"

# The at-java wrapper takes an optional K. Without it the target, the recipe and the wrapper are what they were.
make -n -C "$here/../java" build/at/at-java >"$tmp/recipe.default" 2>&1
make -n -C "$here/../java" build/at/at-java-k50 AT_K=50 >"$tmp/recipe.k50" 2>&1
expect "at-java without K: the wrapper is exactly the old one (the caller's arguments, nothing else)" 0 grep -Fq "langat.Main \"\$@\"\\n' \"" "$tmp/recipe.default"
expect "at-java without K: the wrapper does not mention --k" 0 sh -c "! grep -q -e '--k' '$tmp/recipe.default'"
expect "at-java with AT_K=50: the wrapper appends --k=50 after the caller's arguments" 0 grep -Fq "langat.Main \"\$@\" --k=50\\n' \"" "$tmp/recipe.k50"
expect "at-java without K: the target is build/at/at-java" 0 sh -c "$jmake at-native | grep -q 'run-checks.sh java build/at/at-java 50\$'"
expect "at-java with an empty K is the default wrapper" 0 sh -c "$jmake at-native AT_K= | grep -q 'run-checks.sh java build/at/at-java 50\$'"
expect "at-java with AT_K=50: its own wrapper build/at/at-java-k50, so a stale default one is never reused" 0 sh -c "$jmake at-native AT_K=50 | grep -q 'run-checks.sh java build/at/at-java-k50 50\$'"
expect "at-native with AT_K=50 sets LANG_AT_K for the self-check" 0 sh -c "$jmake at-native AT_K=50 | grep -q 'LANG_AT_K=50 '"
expect "at-native without K does not set LANG_AT_K" 0 sh -c "! $jmake at-native | grep -q LANG_AT_K"
expect "at-profile-native with AT_K=50 sets LANG_AT_K and uses the K wrapper" 0 sh -c "$jmake at-profile-native AT_K=50 | grep 'run-java-profile-checks.sh profiles.tsv' | grep -q 'LANG_AT_K=50 .*at-java-k50'"
expect "at-extra-native runs the self-check under a-prime with the K wrapper" 0 sh -c "$jmake at-extra-native AT_K=50 | grep 'run-java-profile-checks.sh profiles-extra.tsv' | grep -q 'LANG_AT_K=50 .*at-java-k50'"
expect "a K that is not a number is refused by the Makefile" 2 make -n -C "$here/../java" at-native AT_K=lots
expect "a K of 0 is refused by the Makefile" 2 make -n -C "$here/../java" at-native AT_K=0
expect "a K of 5001 is refused by the Makefile (above the fixture's cap)" 2 make -n -C "$here/../java" at-native AT_K=5001
expect "a K of 5000 is accepted by the Makefile" 0 make -n -C "$here/../java" at-native AT_K=5000
expect "a K with a leading zero (007) is refused by the Makefile" 2 make -n -C "$here/../java" at-native AT_K=007
# make itself drops the whitespace before a command-line value, so a lone space is an empty value (the default run) and
# " 50" is 50; a trailing space is kept by make and refused here.
expect "a K that is one space is make's empty value: the default run" 0 sh -c "$jmake at-native 'AT_K= ' | grep -q 'run-checks.sh java build/at/at-java 50\$'"
expect "a K with a trailing space is refused by the Makefile" 2 make -n -C "$here/../java" at-native "AT_K=50 "
expect "a K with a leading space is make's own 50" 0 sh -c "$jmake at-native 'AT_K= 50' | grep -q 'LANG_AT_K=50 '"
expect "a K with a quote and a command is refused by the Makefile, and runs nothing" 2 make -n -C "$here/../java" at-native "AT_K=5';touch $tmp/pwned;'"
expect "the refused quoted K ran nothing" 1 test -e "$tmp/pwned"
# An exported AT_K (environment origin) is ignored: only the command line sets it. The old one-letter K is not read at all.
expect "an exported AT_K is ignored (the default wrapper)" 0 sh -c "AT_K=50 $jmake at-native | grep -q 'run-checks.sh java build/at/at-java 50\$'"
expect "an exported AT_K does not set LANG_AT_K" 0 sh -c "! AT_K=50 $jmake at-native | grep -q LANG_AT_K"
expect "an exported bad AT_K is ignored, not an error" 0 sh -c "AT_K=lots $jmake at-native"
expect "an exported AT_K does not break the wrapper target the build record builds" 0 sh -c "AT_K=5 $jmake build/at/at-java"
expect "an exported K is not read" 0 sh -c "K=5 $jmake build/at/at-java"
expect "a command-line AT_K wins over an exported one" 0 sh -c "AT_K=7 $jmake at-native AT_K=50 | grep -q 'run-checks.sh java build/at/at-java-k50 50\$'"

# run-java-profile-checks.sh: run-checks.sh once per profile, with that profile's flags in JAVA_FLAGS.
rc_stub="$tmp/rc-stub"
cat >"$rc_stub" <<'EOF2'
#!/bin/sh
echo "$JAVA_FLAGS|$*" >>"$RC_LOG"
[ "${RC_FAIL:-}" = b ] && case $JAVA_FLAGS in *TieredStopAtLevel*) exit 1 ;; esac
exit 0
EOF2
chmod +x "$rc_stub"
: >"$tmp/rc.log"
expect "run-java-profile-checks passes when every profile passes" 0 env RC_LOG="$tmp/rc.log" RUN_CHECKS="$rc_stub" sh "$here/run-java-profile-checks.sh" "$profiles_tsv" /x/at-java 7
expect "run-java-profile-checks runs arm a with its flags, the language, the artifact and the run count" 0 grep -Fq -- '-Xint -XX:+UseSerialGC -XX:ActiveProcessorCount=1 -XX:-UsePerfData -Xshare:off|java /x/at-java 7' "$tmp/rc.log"
expect "run-java-profile-checks runs arm b with its flags" 0 grep -q -- '^-XX:TieredStopAtLevel=1 .*|java /x/at-java 7$' "$tmp/rc.log"
expect "run-java-profile-checks fails when one profile fails" 1 env RC_FAIL=b RC_LOG="$tmp/rc.log" RUN_CHECKS="$rc_stub" sh "$here/run-java-profile-checks.sh" "$profiles_tsv" /x/at-java
expect "run-java-profile-checks still runs the other profiles after a failure" 0 sh -c "env RC_FAIL=b RC_LOG='$tmp/rc2.log' RUN_CHECKS='$rc_stub' sh '$here/run-java-profile-checks.sh' '$profiles_tsv' /x/at-java; [ \$(wc -l <'$tmp/rc2.log') -eq 2 ]"
expect "run-java-profile-checks with no profile fails" 1 env RC_LOG="$tmp/rc.log" RUN_CHECKS="$rc_stub" sh "$here/run-java-profile-checks.sh" "$tmp/no_profiles.tsv" /x/at-java
expect "run-java-profile-checks without arguments is a usage error" 64 sh "$here/run-java-profile-checks.sh"

# check-window-java.sh: the window in the bytecode (javap -c -p text of langat.Race): a load of the counter, one call of
# spin, a store of the counter, and nothing else that calls, locks or allocates between them.
jw_good="$tmp/jw_good.txt"
cat >"$jw_good" <<'EOF'
Compiled from "Race.java"
final class langat.Race {
  private void racyIterations();
    Code:
       0: aload_0
       1: getfield      #38                 // Field mode:Llangat/Mode;
       4: invokevirtual #63                 // Method langat/Mode.window:()I
       7: istore_1
       8: iconst_0
       9: istore_2
      10: iload_2
      11: aload_0
      12: getfield      #42                 // Field iterations:I
      15: if_icmpge     51
      18: aload_0
      19: getfield      #69                 // Field counter:J
      22: lstore_3
      23: iload_1
      24: invokestatic  #73                 // Method spin:(I)J
      27: lstore        5
      29: aload_0
      30: lload_3
      31: lconst_1
      32: ladd
      33: putfield      #69                 // Field counter:J
      36: lload         5
      38: putstatic     #77                 // Field sink:J
      41: lconst_1
      42: invokestatic  #80                 // Method hairspring/Hairspring.work:(J)V
      45: iinc          2, 1
      48: goto          10
      51: return

  private void lockedIterations();
    Code:
       0: aload_0
       1: getfield      #38                 // Field mode:Llangat/Mode;
       4: invokevirtual #63                 // Method langat/Mode.window:()I
       7: istore_1
      18: aload_0
      19: getfield      #7                  // Field lock:Ljava/lang/Object;
      22: dup
      23: astore        5
      25: monitorenter
      26: aload_0
      27: getfield      #69                 // Field counter:J
      30: lstore        6
      32: iload_1
      33: invokestatic  #73                 // Method spin:(I)J
      36: lstore_3
      37: aload_0
      38: lload         6
      40: lconst_1
      41: ladd
      42: putfield      #69                 // Field counter:J
      45: aload         5
      47: monitorexit
      48: goto          60
      60: return
}
EOF
expect "check-window accepts a load, one spin call and a store" 0 sh "$here/check-window-java.sh" "$jw_good"
expect "check-window shows the window's instructions" 0 sh -c "sh '$here/check-window-java.sh' '$jw_good' | grep -q 'invokestatic.*Method spin'"
sed 's/24: invokestatic  #73                 \/\/ Method spin:(I)J/24: invokestatic  #99                 \/\/ Method java\/lang\/Thread.yield:()V/' "$jw_good" >"$tmp/jw_other_call.txt"
expect "check-window rejects another call in the window" 1 sh "$here/check-window-java.sh" "$tmp/jw_other_call.txt"
sed 's/23: iload_1/23: invokestatic  #98                 \/\/ Method hairspring\/Hairspring.work:(J)V/' "$jw_good" >"$tmp/jw_sdk.txt"
expect "check-window rejects an SDK call between the load and the store" 1 sh "$here/check-window-java.sh" "$tmp/jw_sdk.txt"
sed 's/23: iload_1/23: new           #97                 \/\/ class java\/lang\/Object/' "$jw_good" >"$tmp/jw_new.txt"
expect "check-window rejects an allocation in the window" 1 sh "$here/check-window-java.sh" "$tmp/jw_new.txt"
sed 's/22: lstore_3/22: monitorenter/' "$jw_good" >"$tmp/jw_monitor.txt"
expect "check-window rejects a monitor operation in the unlocked window" 1 sh "$here/check-window-java.sh" "$tmp/jw_monitor.txt"
sed 's/24: invokestatic  #73                 \/\/ Method spin:(I)J/24: nop/' "$jw_good" >"$tmp/jw_nospin.txt"
expect "check-window rejects a window with no spin call" 1 sh "$here/check-window-java.sh" "$tmp/jw_nospin.txt"
sed 's/25: monitorenter/25: nop/' "$jw_good" >"$tmp/jw_nolock.txt"
expect "check-window rejects a locked mode with no monitor" 1 sh "$here/check-window-java.sh" "$tmp/jw_nolock.txt"
grep -v 'lockedIterations' "$jw_good" >"$tmp/jw_nomethod.txt"
expect "check-window rejects a class without the locked method" 1 sh "$here/check-window-java.sh" "$tmp/jw_nomethod.txt"
expect "check-window rejects an empty file" 1 sh "$here/check-window-java.sh" "$clean"
expect "check-window rejects a missing file" 1 sh "$here/check-window-java.sh" "$tmp/none.txt"
expect "check-window without a file is a usage error" 64 sh "$here/check-window-java.sh"

# check-svc-window.sh: the order of the steps in the service fixture's Store.handle (a text check of the source).
sw_good="$here/../java/svc/src/langsvc/Store.java"
expect "check-svc-window accepts the service fixture's Store.java" 0 sh "$here/check-svc-window.sh" "$sw_good"
awk '/\} else \{/ {r = 1} r && /acc = validate/ {v = $0; next} r && /^            apply\(t\);/ {a = $0; next} r && /applied\.put/ {print; print v; print a; r = 0; next} {print}' "$sw_good" >"$tmp/sw_reordered.java"
expect "check-svc-window rejects a racy branch that records the key before validating" 1 sh "$here/check-svc-window.sh" "$tmp/sw_reordered.java"
awk '/\} else \{/ {r = 1} r && /^            apply\(t\);/ {a = $0; next} r && /applied\.put/ {print; print a; r = 0; next} {print}' "$sw_good" >"$tmp/sw_put_before_apply.java"
expect "check-svc-window rejects a racy branch that records the key before applying" 1 sh "$here/check-svc-window.sh" "$tmp/sw_put_before_apply.java"
sed 's/if (applied.putIfAbsent(t.key(), CLAIMED) != null) {/if (applied.containsKey(t.key())) {/' "$sw_good" >"$tmp/sw_fixed_racy.java"
expect "check-svc-window rejects a fixed branch that checks with containsKey (not atomic)" 1 sh "$here/check-svc-window.sh" "$tmp/sw_fixed_racy.java"
grep -v 'applied.containsKey(t.key())' "$sw_good" >"$tmp/sw_nocheck.java"
expect "check-svc-window rejects a racy branch with no check" 1 sh "$here/check-svc-window.sh" "$tmp/sw_nocheck.java"
grep -v 'mode.atomicClaim()' "$sw_good" >"$tmp/sw_nobranch.java"
expect "check-svc-window rejects a handle() without the two branches" 1 sh "$here/check-svc-window.sh" "$tmp/sw_nobranch.java"
expect "check-svc-window rejects an empty file" 1 sh "$here/check-svc-window.sh" "$clean"
expect "check-svc-window rejects a missing file" 1 sh "$here/check-svc-window.sh" "$tmp/none.java"
expect "check-svc-window without a file is a usage error" 64 sh "$here/check-svc-window.sh"

# build-record-java-svc.sh: a JDK that is not the pinned build fails the record and stages no artifact (no real JDK needed:
# the fake java reports another build and there is no javac, so the builds fail too).
mkdir -p "$tmp/svcjdk/bin"
printf '#!/bin/sh\necho "openjdk version \\"21.0.13\\" 2026-10-20 LTS" >&2\necho "OpenJDK Runtime Environment Temurin-21.0.13+11 (build 21.0.13+11-LTS)" >&2\n' >"$tmp/svcjdk/bin/java"
chmod +x "$tmp/svcjdk/bin/java"
expect "build-record-java-svc fails with a JDK that is not the pin" 1 env JAVA_HOME="$tmp/svcjdk" ARTIFACT_DIR="$tmp/svc-artifact" sh "$here/build-record-java-svc.sh" "$tmp/svc-record.md"
expect "build-record-java-svc's record says the JDK check failed and RESULT FAIL" 0 sh -c "grep -qx 'FAIL the JDK is the pinned build' '$tmp/svc-record.md' && grep -qx 'RESULT FAIL' '$tmp/svc-record.md'"
expect "build-record-java-svc stages no artifact when a check failed" 1 test -e "$tmp/svc-artifact"
expect "build-record-java-svc without an output file is a usage error" 64 sh "$here/build-record-java-svc.sh"

# The .NET fixture: its constants and start barrier are read from its source, its sources read nothing forbidden, and
# run-dotnet-profile-checks.sh runs the self-check once per environment profile arm (here with a stub checker).
dotnet_fixture="$fixtures_dir/dotnet"
expect "constants evidence passes for the .NET fixture" 0 sh "$evidence" constants dotnet "$dotnet_fixture/at/src/Contract.cs"
sed 's/Window = 1000;/Window = 2000;/' "$dotnet_fixture/at/src/Contract.cs" >"$tmp/Contract_w2000.cs"
expect "a .NET window of 2000 is rejected" 1 sh "$evidence" constants dotnet "$tmp/Contract_w2000.cs"
sed 's/WindowWide = 10000;/WindowWide = 20000;/' "$dotnet_fixture/at/src/Contract.cs" >"$tmp/Contract_wide.cs"
expect "a .NET wide window of 20000 is rejected" 1 sh "$evidence" constants dotnet "$tmp/Contract_wide.cs"
sed 's/Iterations = 1000;/Iterations = 999;/' "$dotnet_fixture/at/src/Contract.cs" >"$tmp/Contract_k999.cs"
expect "a .NET iteration count of 999 is rejected" 1 sh "$evidence" constants dotnet "$tmp/Contract_k999.cs"
expect "the real .NET fixture has a start barrier" 0 sh "$evidence" barrier "$dotnet_fixture/at/src/Race.cs"
expect "the .NET fixture without its barrier wait is rejected" 1 sh "$evidence" barrier "$(nowait dotnet "$dotnet_fixture/at/src/Race.cs" '_barrier.Await()')"
expect "the real .NET fixture reads nothing forbidden" 0 sh "$here/check-sources-dotnet.sh" "$dotnet_fixture/at/src"
bad_dotnet="$tmp/Bad.cs"
printf 'class Bad { long t = System.Diagnostics.Stopwatch.GetTimestamp(); }\n' >"$bad_dotnet"
expect "a .NET fixture that reads the clock is rejected" 1 sh "$here/check-sources-dotnet.sh" "$bad_dotnet"

rcd_stub="$tmp/rcd-stub"
cat >"$rcd_stub" <<'EOF2'
#!/bin/sh
# The wrapper the profile script writes is the artifact: print what it would run and the variables it adds.
echo "$(sed -n 's/^exec env //p' "$2")|$*" >>"$RC_LOG"
[ "${RC_FAIL:-}" = b ] && grep -q TieredCompilation "$2" && exit 1
exit 0
EOF2
chmod +x "$rcd_stub"
: >"$tmp/rcd.log"
dotnet_manifest="$dotnet_fixture/profiles.manifest"
expect "run-dotnet-profile-checks passes when both arms pass" 0 env RC_LOG="$tmp/rcd.log" RUN_CHECKS="$rcd_stub" sh "$here/run-dotnet-profile-checks.sh" "$dotnet_manifest" /x/LangAt.dll /bin/sh 7
expect "run-dotnet-profile-checks runs arm a with DOTNET_ROOT only, the language and the run count" 0 grep -q "^'DOTNET_ROOT=[^']*' '/bin/sh' '/x/LangAt.dll' \"\$@\"|dotnet .*/at-dotnet-a 7\$" "$tmp/rcd.log"
expect "run-dotnet-profile-checks runs arm b with the four extra variables" 0 grep -q "TieredCompilation=0.*EnableWriteXorExecute=0.*gcServer=0.*PROCESSOR_COUNT=1.*at-dotnet-b 7\$" "$tmp/rcd.log"
expect "run-dotnet-profile-checks fails when arm b fails" 1 env RC_FAIL=b RC_LOG="$tmp/rcd.log" RUN_CHECKS="$rcd_stub" sh "$here/run-dotnet-profile-checks.sh" "$dotnet_manifest" /x/LangAt.dll /bin/sh
expect "run-dotnet-profile-checks still runs arm b after arm a fails" 0 sh -c "env RC_FAIL=a RC_LOG='$tmp/rcd2.log' RUN_CHECKS='$rcd_stub' sh '$here/run-dotnet-profile-checks.sh' '$dotnet_manifest' /x/LangAt.dll /bin/sh; [ \$(wc -l <'$tmp/rcd2.log') -eq 2 ]"
expect "run-dotnet-profile-checks with a bad manifest fails" 1 env RC_LOG="$tmp/rcd.log" RUN_CHECKS="$rcd_stub" sh "$here/run-dotnet-profile-checks.sh" "$tmp/no_manifest" /x/LangAt.dll /bin/sh
expect "run-dotnet-profile-checks without arguments is a usage error" 64 sh "$here/run-dotnet-profile-checks.sh"

# check-dotnet-jit.sh: what the JIT made of the two worker loops. A fake dotnet prints the summary and the listings the real
# one would (arm64 or x86-64 style), and each fake breaks exactly one thing the check looks for.
jit_dotnet="$tmp/jit-dotnet"
cat >"$jit_dotnet" <<'EOF2'
#!/bin/sh
# usage: jit-dotnet <dll> <mode>; FAKE picks the listing, FAKE_ARCH arm64 (default) or x64. The JIT file is $DOTNET_JitStdOutFile.
mode=$2
out=$DOTNET_JitStdOutFile
want=RacyIterations
[ "$mode" = fixed ] && want=LockedIterations
f=${FAKE:-}
arch=${FAKE_ARCH:-arm64}
[ "$f" = exit1 ] && exit 1
[ "$f" = silent ] && exit 0
{
  echo "   1: JIT compiled LangAt.Program:Main(System.String[]) [Tier0, IL size=152, code size=1036]"
  [ "$f" = nospin ] || echo "   2: JIT compiled LangAt.Race:Spin(int):ulong [Tier0, IL size=40, code size=200]"
  echo "   3: JIT compiled LangAt.Race:$want() [${TIER:-Instrumented Tier0}, IL size=65, code size=284]"
  [ "$f" = twocompiles ] && echo "   4: JIT compiled LangAt.Race:$want() [Tier1-OSR @0x30, IL size=65, code size=200]"
  [ "$f" = otherrun ] && echo "   5: JIT compiled LangAt.Race:RacyIterations() [Tier0, IL size=65, code size=284]" && [ "$mode" = fixed ] || true
  echo
  [ "$f" = nolisting ] && exit 0
  emit() { # emit <load> <call> <store> <tier>
    echo "; Assembly listing for method LangAt.Race:$want():this ($4)"
    echo "G_M000_IG02:                ;; offset=0x0018"
    if [ "$arch" = arm64 ]; then
      echo "            ldr     x0, [fp, #0x38]"
      echo "            ldr     w1, [x0, #0x38]"
      echo "            $1"
      echo "            $2"
      echo "            $3"
    else
      echo "            mov      rax, qword ptr [rbp-0x20]"
      echo "            $1"
      echo "            $2"
      echo "            $3"
    fi
    echo
  }
  if [ "$arch" = arm64 ]; then
    L="ldr     x0, [x0, #0x30]"; C="blr     x1"; S="str     x0, [x1, #0x30]"
    case $f in
      nostore) S="add     x0, x0, #1" ;;
      nocall) C="nop" ;;
      twocalls) C="blr     x1
            bl      CORINFO_HELP_COUNTPROFILE32" ;;
      namedother) C="bl      LangAt.Race:Other(int):ulong" ;;
      otheroffset) S="str     x0, [x1, #0x40]" ;;
      noload) L="ldr     x0, [fp, #0x30]" ;;
      narrow) L="ldr     w0, [x0, #0x30]" ;;
      storebeforeload) ;;
    esac
    [ "${TIER:-}" = FullOpts ] && C="bl      LangAt.Race:Spin(int):ulong"
  else
    L="mov      rax, qword ptr [rbx+0x30]"; C="call     [LangAt.Race:Spin(int):ulong]"; S="mov      qword ptr [rbx+0x30], rax"
    case $f in
      nostore) S="add      rax, 1" ;;
      nocall) C="nop" ;;
      twocalls) C="call     [LangAt.Race:Spin(int):ulong]
            call     [CORINFO_HELP_POLL_GC]" ;;
      namedother) C="call     [System.Threading.Thread:Yield():bool]" ;;
      otheroffset) S="mov      qword ptr [rbx+0x40], rax" ;;
      noload) L="mov      rax, qword ptr [rbp+0x30]" ;;
      narrow) L="mov      eax, dword ptr [rbx+0x30]" ;;
    esac
  fi
  if [ "$f" = badsecond ] || [ "$f" = twocompiles ]; then
    emit "$L" "$C" "$S" "${TIER:-Instrumented Tier0}"
    [ "$f" = twocompiles ] && emit "ldr     x0, [x0, #0x30]" "blr     x1" "str     x0, [x1, #0x30]" "Tier1-OSR" && exit 0
    emit "$L" "nop" "$S" "Tier1"
    exit 0
  fi
  emit "$L" "$C" "$S" "${TIER:-Instrumented Tier0}"
} >"$out"
exit 0
EOF2
chmod +x "$jit_dotnet"
jit_manifest="$fixtures_dir/dotnet/profiles.manifest"
cdj() { # cdj <name> <wanted exit> <FAKE value> [FAKE_ARCH] [TIER]
  expect "check-dotnet-jit $1" "$2" env FAKE="$3" FAKE_ARCH="${4:-arm64}" TIER="${5:-}" sh "$here/check-dotnet-jit.sh" "$jit_manifest" "$tmp/fake.dll" "$jit_dotnet"
}
cdj "accepts an arm64 window under Instrumented Tier0" 0 ""
cdj "accepts an arm64 window under FullOpts, where the call is named" 0 "" arm64 FullOpts
cdj "accepts an x86-64 window" 0 "" x64
cdj "accepts an OSR compile after a tier 0 compile when both show the window" 0 twocompiles
cdj "rejects a listing with no store of the loaded field" 1 nostore
cdj "rejects a listing with no call between the load and the store" 1 nocall
cdj "rejects a helper call inside the window" 1 twocalls
cdj "rejects a call to something other than Spin inside the window" 1 namedother
cdj "rejects a store to another offset than the load" 1 otheroffset
cdj "rejects a load from the stack frame as the counter" 1 noload
cdj "rejects a 32-bit load as the counter" 1 narrow
cdj "rejects a later recompile that lost the call" 1 badsecond
cdj "rejects an x86-64 listing with no store" 1 nostore x64
cdj "rejects an x86-64 listing with no call" 1 nocall x64
cdj "rejects an x86-64 helper call inside the window" 1 twocalls x64
cdj "rejects an x86-64 call to another method inside the window" 1 namedother x64
cdj "rejects an x86-64 load from the frame" 1 noload x64
cdj "rejects a compilation with no listing (the listing control)" 1 nolisting
cdj "rejects a JIT that printed nothing (the output control)" 1 silent
cdj "rejects an output without Spin among the compilations (the Spin control)" 1 nospin
cdj "rejects the other worker method being compiled in a mode that does not run it" 1 otherrun
cdj "rejects a program that fails" 1 exit1
expect "check-dotnet-jit names the default environment and each arm" 0 sh -c "env FAKE= sh '$here/check-dotnet-jit.sh' '$jit_manifest' '$tmp/fake.dll' '$jit_dotnet' | grep -c '^arm ' | grep -qx 3"
expect "check-dotnet-jit with a missing manifest fails" 1 sh "$here/check-dotnet-jit.sh" "$tmp/none.manifest" "$tmp/fake.dll" "$jit_dotnet"
expect "check-dotnet-jit without arguments is a usage error" 64 sh "$here/check-dotnet-jit.sh"

# summarize-at-native.sh: the one-line summary of a native self-check log, for the manual java-native-k workflow.
mklog() { # mklog <file> <text after "java buggy: "> [extra line]
  printf '%s\n' "java buggy: $2" 'java fixed: property false in 0 of 50 native runs, final exact in 50' 'java wide:  property false in 4 of 10 native runs' ${3:+"$3"} >"$1"
}
sumeq() { # sumeq <name> <wanted output> <log> <k> [profile]
  expect "$1" 0 sh -c '[ "$(sh "$1" "$3" "$4" ${5:+"$5"})" = "$2" ]' _ "$here/summarize-at-native.sh" "$2" "$3" "$4" ${5:+"$5"}
}
sumlog="$tmp/at-native.log"
mklog "$sumlog" 'property false in 37 of 50 native runs'
sumeq "summarize-at-native prints runs, fired and k" "runs=50 fired=37 k=50" "$sumlog" 50
sumeq "summarize-at-native accepts the largest k" "runs=50 fired=37 k=5000" "$sumlog" 5000
sumeq "summarize-at-native accepts k=1" "runs=50 fired=37 k=1" "$sumlog" 1
mklog "$tmp/all.log" 'property false in 5 of 5 native runs'
sumeq "summarize-at-native accepts fired equal to runs" "runs=5 fired=5 k=1" "$tmp/all.log" 1
mklog "$tmp/none.log" 'property false in 0 of 5 native runs'
sumeq "summarize-at-native accepts fired 0" "runs=5 fired=0 k=1" "$tmp/none.log" 1
expect "summarize-at-native reads the log from stdin" 0 sh -c '[ "$(sh "$1" - 7 <"$2")" = "runs=50 fired=37 k=7" ]' _ "$here/summarize-at-native.sh" "$sumlog"
expect "summarize-at-native fails on an empty log" 1 sh "$here/summarize-at-native.sh" /dev/null 50
expect "summarize-at-native fails on a missing log" 1 sh "$here/summarize-at-native.sh" "$tmp/missing.log" 50
printf 'java wide:  property false in 4 of 10 native runs\n' >"$tmp/wide.log"
expect "summarize-at-native does not take a wide line for the buggy one" 1 sh "$here/summarize-at-native.sh" "$tmp/wide.log" 50
printf 'java fixed: property false in 4 of 10 native runs\n' >"$tmp/fixed.log"
expect "summarize-at-native does not take a fixed line for the buggy one" 1 sh "$here/summarize-at-native.sh" "$tmp/fixed.log" 50
mklog "$tmp/fired-gt.log" 'property false in 51 of 50 native runs'
expect "summarize-at-native rejects fired above runs" 1 sh "$here/summarize-at-native.sh" "$tmp/fired-gt.log" 50
mklog "$tmp/runs0.log" 'property false in 0 of 0 native runs'
expect "summarize-at-native rejects 0 runs" 1 sh "$here/summarize-at-native.sh" "$tmp/runs0.log" 50
mklog "$tmp/two.log" 'property false in 37 of 50 native runs' 'java buggy: property false in 1 of 50 native runs'
expect "summarize-at-native rejects two valid buggy lines" 1 sh "$here/summarize-at-native.sh" "$tmp/two.log" 50
mklog "$tmp/two-bad.log" 'property false in 99 of 50 native runs' 'java buggy: property false in 1 of 50 native runs'
expect "summarize-at-native rejects two buggy lines when only the second is valid" 1 sh "$here/summarize-at-native.sh" "$tmp/two-bad.log" 50
mklog "$tmp/two-bad2.log" 'property false in 1 of 50 native runs' 'java buggy: property false in 99 of 50 native runs'
expect "summarize-at-native rejects two buggy lines when only the first is valid" 1 sh "$here/summarize-at-native.sh" "$tmp/two-bad2.log" 50
mklog "$tmp/trail.log" 'property false in 37 of 50 native runs, extra'
expect "summarize-at-native rejects trailing text on the buggy line" 1 sh "$here/summarize-at-native.sh" "$tmp/trail.log" 50
mklog "$tmp/trail0.log" 'property false in 37 of 50 native runs0'
expect "summarize-at-native rejects digits glued to the end of the buggy line" 1 sh "$here/summarize-at-native.sh" "$tmp/trail0.log" 50
printf '%s\n' 'java fixed: property false in 0 of 40 native runs' 'note: java buggy: see above' 'java buggy: property false in 3 of 9 native runs' >"$tmp/order.log"
sumeq "summarize-at-native reports the buggy line wherever it is, and a mention of buggy: mid-line is not a second line" "runs=9 fired=3 k=2" "$tmp/order.log" 2
mklog "$tmp/huge.log" 'property false in 1 of 99999999999999999999 native runs'
expect "summarize-at-native rejects a huge runs count" 1 sh "$here/summarize-at-native.sh" "$tmp/huge.log" 50
expect "summarize-at-native rejects k=0" 64 sh "$here/summarize-at-native.sh" "$sumlog" 0
expect "summarize-at-native rejects k with a leading zero" 64 sh "$here/summarize-at-native.sh" "$sumlog" 050
expect "summarize-at-native rejects k above 5000" 64 sh "$here/summarize-at-native.sh" "$sumlog" 5001
expect "summarize-at-native rejects a very long k" 64 sh "$here/summarize-at-native.sh" "$sumlog" 99999999999999999999
expect "summarize-at-native rejects an empty k" 64 sh "$here/summarize-at-native.sh" "$sumlog" ""
expect "summarize-at-native rejects a k that is not a number" 64 sh "$here/summarize-at-native.sh" "$sumlog" 5x
expect "summarize-at-native without arguments is a usage error" 64 sh "$here/summarize-at-native.sh"

# With a profile: the log of run-java-profile-checks.sh holds one section per arm ("--- arm <name>: <flags>" and the
# self-check's lines); the summary is that arm's alone and names it.
proflog="$tmp/profiles.log"
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 7 of 50 native runs' 'java fixed: property false in 0 of 50 native runs, final exact in 50' 'java wide:  property false in 4 of 10 native runs' 'ok   arm a: the self-check passed under these flags' '--- arm b: -XX:TieredStopAtLevel=1' 'java buggy: property false in 41 of 60 native runs' 'java fixed: property false in 0 of 60 native runs, final exact in 60' >"$proflog"
printf '%s\n' '--- arm a-prime: -Xint -Xshare:auto' 'java buggy: property false in 12 of 50 native runs' 'java fixed: property false in 0 of 50 native runs, final exact in 50' >"$tmp/prime.log"
sumeq "summarize-at-native names profile a-prime and reports its own buggy count" "profile=a-prime runs=50 fired=12 k=50" "$tmp/prime.log" 50 a-prime
sumeq "summarize-at-native reports only profile a when the log also holds b" "profile=a runs=50 fired=7 k=50" "$proflog" 50 a
sumeq "summarize-at-native reports profile b from the same log" "profile=b runs=60 fired=41 k=50" "$proflog" 50 b
expect "summarize-at-native: profile a is not found in a log that has only a-prime" 1 sh "$here/summarize-at-native.sh" "$tmp/prime.log" 50 a
expect "summarize-at-native: a profile absent from the log fails" 1 sh "$here/summarize-at-native.sh" "$proflog" 50 c
printf '%s\n' '--- arm a: -Xint' 'java fixed: property false in 0 of 50 native runs' '--- arm b: -Xint' 'java buggy: property false in 41 of 60 native runs' >"$tmp/nobuggy-a.log"
expect "summarize-at-native: a profile section without a buggy line fails (b's line is not a's)" 1 sh "$here/summarize-at-native.sh" "$tmp/nobuggy-a.log" 50 a
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 1 of 5 native runs' 'java buggy: property false in 2 of 5 native runs' >"$tmp/twobuggy-a.log"
expect "summarize-at-native: two buggy lines in the profile's section fail" 1 sh "$here/summarize-at-native.sh" "$tmp/twobuggy-a.log" 50 a
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 1 of 5 native runs' '--- arm a: -Xint' 'java buggy: property false in 2 of 5 native runs' >"$tmp/twosect-a.log"
expect "summarize-at-native: a profile run twice in one log fails" 1 sh "$here/summarize-at-native.sh" "$tmp/twosect-a.log" 50 a
printf '%s\n' '--- arm a-prime: -Xint -Xshare:auto' 'java buggy: property false in 12 of 50 native runs' '--- arm a: -Xint' 'java buggy: property false in 7 of 50 native runs' >"$tmp/both.log"
sumeq "summarize-at-native: a log with a and a-prime reports a alone" "profile=a runs=50 fired=7 k=50" "$tmp/both.log" 50 a
sumeq "summarize-at-native: a log with a and a-prime reports a-prime alone" "profile=a-prime runs=50 fired=12 k=50" "$tmp/both.log" 50 a-prime
printf '%s\n' '--- arm a-prime: -Xint' 'java buggy: property false in 12 of 50 native runs' >"$tmp/primeonly.log"
expect "summarize-at-native: a section of a-prime is not a section of a" 1 sh "$here/summarize-at-native.sh" "$tmp/primeonly.log" 50 a
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 0 of 50 native runs' 'java fixed: property false in 0 of 50 native runs, final exact in 50' 'FAIL the buggy mode never fired in 50 runs on 4 CPUs: the planted race is not reachable' >"$tmp/zero.log"
sumeq "summarize-at-native: fired=0 with the self-check's FAIL line is a result" "profile=a runs=50 fired=0 k=2" "$tmp/zero.log" 2 a
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 3 of 50 native runs' '--- arm b: -Xint' 'java buggy: property false in 0 of 50 native runs' 'FAIL the buggy mode never fired in 50 runs on 4 CPUs: the planted race is not reachable' >"$tmp/b-zero.log"
sumeq "summarize-at-native: arm b never firing does not hide arm a's result" "profile=a runs=50 fired=3 k=2" "$tmp/b-zero.log" 2 a
printf '%s\n' '--- arm a: -Xint' 'java buggy: property false in 1 of 5 native runs' '--- arm a: -Xint' 'java fixed: property false in 0 of 5 native runs' >"$tmp/twosect2-a.log"
expect "summarize-at-native: a profile run twice fails even when only one run has a buggy line" 1 sh "$here/summarize-at-native.sh" "$tmp/twosect2-a.log" 50 a
expect "summarize-at-native rejects a profile with a space" 64 sh "$here/summarize-at-native.sh" "$proflog" 50 "a b"
expect "summarize-at-native rejects a profile with a shell metacharacter" 64 sh "$here/summarize-at-native.sh" "$proflog" 50 'a;id'
expect "summarize-at-native rejects an empty profile" 64 sh "$here/summarize-at-native.sh" "$proflog" 50 ""
expect "summarize-at-native rejects a profile that starts with a dash" 64 sh "$here/summarize-at-native.sh" "$proflog" 50 -x
expect "summarize-at-native rejects a fourth argument" 64 sh "$here/summarize-at-native.sh" "$proflog" 50 a extra

# The java-native-k workflow: its own file, manual only, read-only, pinned like the java build record. The mutation run
# points JAVA_NATIVE_K_WORKFLOW at mutated copies.
WF=${JAVA_NATIVE_K_WORKFLOW:-$here/../../../.github/workflows/java-native-k.yml}
CI=${CI_WORKFLOW:-$here/../../../.github/workflows/ci.yml}
wfexpect() { # wfexpect <name> <the whole line that must be in the workflow>
  expect "java-native-k.yml: $1" 0 sh -c 'grep -qxF -- "$1" "$2"' _ "$2" "$WF"
}
wfsh() { # wfsh <name> <sh code taking the workflow as $1 (and ci.yml as $2) and succeeding when the property holds>
  expect "java-native-k.yml: $1" 0 sh -c "$2" _ "$WF" "$CI"
}
wfexpect "triggers on workflow_dispatch" "  workflow_dispatch:"
wfsh "workflow_dispatch is the only trigger" '[ "$(awk "/^on:/ {on=1; next} on && /^[^ ]/ {exit} on && /^  [a-z_]+:/ {n++} END {print n}" "$1")" = 1 ] && ! grep -Eq "^on:.+" "$1" && ! grep -Eq "pull_request|^  push:|schedule:|workflow_run|workflow_call" "$1"'
wfsh "top-level permissions are contents: read and nothing else" '[ "$(awk "/^permissions:/ {p=1; next} p && /^[^ ]/ {exit} p {print}" "$1")" = "  contents: read" ] && [ "$(grep -c "permissions:" "$1")" = 1 ]'
wfsh "no write permission anywhere" '! grep -Eq "write|read-all|id-token" "$1"'
wfexpect "runs on ubuntu-24.04" "    runs-on: ubuntu-24.04"
wfexpect "has a 45 minute timeout" "    timeout-minutes: 45"
wfexpect "uses the pinned Temurin" '          java-version: "21.0.12+101.0.LTS"'
wfexpect "uses Temurin" "          distribution: temurin"
wfsh "the pinned Temurin equals the java build record's in ci.yml" 'a=$(grep -m1 "java-version: .*+" "$1"); b=$(grep -m1 "java-version: .*+" "$2"); [ -n "$a" ] && [ "$a" = "$b" ]'
wfsh "every action is pinned to @v4 (three uses)" '[ "$(grep -c "uses:" "$1")" = 3 ] && [ "$(grep -c "uses: actions/[a-z-]*@v4$" "$1")" = 3 ]'
wfexpect "the digits-only, no-leading-zero check" '              "" | *[!0-9]* | 0*) echo "$1='"'"'$2'"'"' is not a number from 1 to $3 (digits only, no leading zero)" >&2; exit 1 ;;'
wfexpect "the range check" '            [ "${#2}" -le 4 ] && [ "$2" -le "$3" ] || { echo "$1='"'"'$2'"'"' is above $3" >&2; exit 1; }'
wfexpect "at_k is bounded at 5000" '          check at_k "$AT_K" 5000'
wfexpect "runs is bounded at 200" '          check runs "$RUNS" 200'
wfexpect "the check step has the id the later steps read" "        id: inputs"
wfsh "the input check is the first step" '[ "$(awk "/^    steps:/ {s=1; next} s && /^      - / {print; exit}" "$1")" = "      - name: Check the inputs" ]'
wfsh "no expression in any run: block or one-line run:" '! awk "/^ +run: *[|>]/ {r=1; next} /^ +[a-z-]+:|^ +- / {r=0} r" "$1" | grep -q "\${{" && ! grep -Eq "^ +run: .*\\$\{\{" "$1"'
wfsh "the raw inputs are read only by the check step's env (four uses)" '[ "$(grep -c "\${{ inputs\." "$1")" = 4 ]'
wfsh "the later steps read the checked outputs" '[ "$(grep -c "\${{ steps.inputs.outputs.at_k }}" "$1")" = 3 ] && [ "$(grep -c "\${{ steps.inputs.outputs.runs }}" "$1")" = 1 ] && [ "$(grep -c "\${{ steps.inputs.outputs.profile }}" "$1")" = 3 ]'
wfexpect "profile is a choice input" "        type: choice"
wfexpect "the choice offers a-prime" "          - a-prime"
wfexpect "the choice offers a" "          - a"
wfsh "the choice offers only a-prime and a, and defaults to a-prime" '[ "$(awk "/^      profile:/ {p=1; next} p && /^      [a-z_]+:/ {exit} p && /^          - / {print}" "$1" | tr "\n" ";")" = "          - a-prime;          - a;" ] && awk "/^      profile:/ {p=1; next} p && /^      [a-z_]+:/ {exit} p" "$1" | grep -qx "        default: a-prime"'
wfexpect "the check step reads the profile input" '          PROFILE: ${{ inputs.profile }}'
wfexpect "the profile is checked against the two names" '          case "$PROFILE" in a-prime | a) ;; *) echo "profile='"'"'$PROFILE'"'"' is not a-prime or a" >&2; exit 1 ;; esac'
wfexpect "the checked values are the outputs" "          printf 'at_k=%s\\nruns=%s\\nprofile=%s\\nfixture=%s\\n' \"\$AT_K\" \"\$RUNS\" \"\$PROFILE\" \"\${FIXTURE:-at}\" >> \"\$GITHUB_OUTPUT\""
wfsh "the profile is checked before the outputs are written" '[ "$(grep -n "case \"\$PROFILE\" in a-prime | a)" "$1" | cut -d: -f1)" -lt "$(grep -n "GITHUB_OUTPUT" "$1" | cut -d: -f1)" ]'
wfexpect "profile a-prime runs the fixture's extra-native target" "            a-prime) target=\$FIXTURE-extra-native ;;"
wfexpect "profile a runs the fixture's profile-native target" "            a) target=\$FIXTURE-profile-native ;;"
wfexpect "any other profile is refused in the run step" '            *) echo "unknown profile $PROFILE" >&2; exit 1 ;;'
wfexpect "the make line: CPUs 1-3, low priority, the profile's target, AT_K and RUNS" '          taskset -c 1-3 nice -n 10 make -C examples/language-fixtures/java "$target" AT_K="$AT_K" RUNS="$RUNS" 2>&1 | tee -a "$RUNNER_TEMP/java-native-k.log"'
wfexpect "the summary names the profile" '          summary=$(sh examples/language-fixtures/scripts/summarize-at-native.sh "$RUNNER_TEMP/java-native-k.log" "$AT_K" "$PROFILE") || { echo "RUN FAILED: the log has no valid summary line. This is not a result." | tee -a "$GITHUB_STEP_SUMMARY"; exit 1; }'
wfsh "the run and summary steps get the profile from the step output" '[ "$(grep -c "PROFILE: \${{ steps.inputs.outputs.profile }}" "$1")" = 2 ]'
wfexpect "a failed make fails the step (pipefail)" "          set -o pipefail"
wfexpect "builds the chosen fixture first" '        run: make -C examples/language-fixtures/java "$FIXTURE-jar"'
wfexpect "fixture is a choice input defaulting to at" "        default: at"
wfsh "the fixture choice offers only at and svc" '[ "$(awk "/^      fixture:/ {p=1; next} p && /^      [a-z_]+:/ {exit} p && /^          - / {print}" "$1" | tr "\n" ";")" = "          - at;          - svc;" ]'
wfexpect "the fixture is checked against the two names" '          case "$FIXTURE" in at | svc | "") ;; *) echo "fixture='"'"'$FIXTURE'"'"' is not at or svc" >&2; exit 1 ;; esac'
wfsh "the later steps read the checked fixture (build, record dir name, native, summary, artifact name)" '[ "$(grep -c "\${{ steps.inputs.outputs.fixture }}" "$1")" = 4 ] && [ "$(grep -c "if: steps.inputs.outputs.fixture == .svc." "$1")" = 1 ]'
wfexpect "the service fixture's record script runs only for svc" '          sh examples/language-fixtures/scripts/build-record-java-svc.sh "$RUNNER_TEMP/svc-artifact-record.md" || status=$?'
wfexpect "the record reaches the step summary even when the record script fails" '          if [ -f "$RUNNER_TEMP/svc-artifact-record.md" ]; then cat "$RUNNER_TEMP/svc-artifact-record.md" >> "$GITHUB_STEP_SUMMARY"; fi'
wfexpect "a failed record script still fails the step" '          exit "$status"'
wfsh "exactly the Summary and Upload steps run if always()" '[ "$(awk "/^      - /{name=\$0} /^        if: always\(\)$/ {print name}" "$1" | tr "\n" ";")" = "      - name: Summary;      - name: Upload the log;" ]'
wfexpect "the native step has an id" "        id: native"
wfexpect "the summary reads the outcome of the native step" '          NATIVE_OUTCOME: ${{ steps.native.outcome }}'
wfsh "the outcome is read once, by the Summary step" '[ "$(grep -c "steps.native.outcome" "$1")" = 1 ]'
wfsh "the job status no longer short-circuits the summary" '! grep -q "JOB_STATUS\|job.status" "$1"'
wfexpect "a native step that did not run is not a result" '            *) echo "RUN FAILED (native runs: ${NATIVE_OUTCOME:-not run}). This is not a result." | tee -a "$GITHUB_STEP_SUMMARY"; exit 0 ;;'
wfexpect "only a finished native step (success or failure) has its log read" "            success | failure) ;;"
wfexpect "a log with no valid summary line is not a result and fails the step" '          summary=$(sh examples/language-fixtures/scripts/summarize-at-native.sh "$RUNNER_TEMP/java-native-k.log" "$AT_K" "$PROFILE") || { echo "RUN FAILED: the log has no valid summary line. This is not a result." | tee -a "$GITHUB_STEP_SUMMARY"; exit 1; }'
wfexpect "the summary line carries the make outcome" '          line="$summary (make self-check: $NATIVE_OUTCOME)"'
wfexpect "the summary lists the FAIL lines of the log" '            grep "^FAIL" "$RUNNER_TEMP/java-native-k.log" || true'
wfexpect "the summary says fired=0 is a result and the make failure is the self-check's rule" '              echo "fired=0 is a result. The make failure is the self-check'"'"'s own rule (for example the buggy mode never fired); the FAIL lines above say which."'
wfexpect "the artifact name is a fixed prefix, the checked fixture, profile and K" "          name: java-native-k-\${{ steps.inputs.outputs.fixture }}-\${{ steps.inputs.outputs.profile }}-\${{ steps.inputs.outputs.at_k }}"
wfexpect "a missing log only warns" "          if-no-files-found: warn"
wfsh "uses no secrets" '! grep -Eq "[$][{][{] *secrets|^ *secrets:" "$1"'
wfsh "ci.yml has no java-native-k job and no dispatch inputs" '! grep -Eq "java-native-k|^    inputs:" "$2"'

# The pinned .NET SDK. Two files: the root global.json is a floor for everyone who builds in the repository (any .NET 10 SDK of
# feature band 100 or later is accepted); scripts/record-global.json is the exact pin for the build records, the single source of
# the pinned version. The record scripts read it, copy it as global.json into the directories they build from, and CI installs it.
# The fake dotnet behaves like the real one about global.json: it looks for the file from the working directory upwards; with
# "rollForward": "disable" it reports exactly the pinned version if that is installed (FAKE_INSTALLED, a list) and fails otherwise;
# with a floor it reports the newest installed SDK (FAKE_NEWEST) if that is not below the floor; with no global.json it reports
# the newest installed SDK. Everything else fails, so a record run stops after the checks looked at here.
pin_here=$(cd "$here" && pwd)
repo_root=$(cd "$here/../../.." && pwd)
record_pin="$repo_root/scripts/record-global.json"
pin_dotnet="$tmp/pin_dotnet"
cat >"$pin_dotnet" <<'PINEOF'
#!/bin/sh
case ${1:-} in
  --list-runtimes) echo "Microsoft.NETCore.App 10.0.13 [/fake]"; exit 0 ;;
  --version) ;;
  *) exit 1 ;;
esac
d=$(pwd)
while [ "$d" != "/" ]; do
  if [ -f "$d/global.json" ]; then
    flat=$(tr -d '\n' <"$d/global.json")
    want=$(printf '%s' "$flat" | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    roll=$(printf '%s' "$flat" | sed -n 's/.*"rollForward"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
    if [ "$roll" = disable ]; then
      for have in $FAKE_INSTALLED; do [ "$have" = "$want" ] && { echo "$want"; exit 0; }; done
    else
      low=$(printf '%s\n%s\n' "$want" "$FAKE_NEWEST" | sort -V | head -1)
      [ "$low" = "$want" ] && { echo "$FAKE_NEWEST"; exit 0; }
    fi
    echo "A compatible .NET SDK was not found. Requested SDK version: $want" >&2
    exit 145
  fi
  d=$(dirname "$d")
done
echo "$FAKE_NEWEST"
PINEOF
chmod +x "$pin_dotnet"
# pin_record <dir to run in> <installed SDKs> <newest installed> [VAR=value...]: runs the record named by PIN_SCRIPT (default the
# .NET fixture's) with the fake dotnet from that directory; the record text is left in $tmp/pin_record.md.
pin_record() {
  pr_dir=$1
  pr_installed=$2
  pr_newest=$3
  shift 3
  rm -f "$tmp/pin_record.md"
  (cd "$pr_dir" && env DOTNET="$pin_dotnet" FAKE_INSTALLED="$pr_installed" FAKE_NEWEST="$pr_newest" RUNTIME_SOURCE=installed "$@" \
    sh "$pin_here/${PIN_SCRIPT:-build-record-dotnet.sh}" "$tmp/pin_record.md" >/dev/null 2>&1)
}
pin_has() { grep -qF -- "$1" "$tmp/pin_record.md"; }
# pin_flat <file>: the file on one line without spaces.
pin_flat() { tr -d ' \n' <"$1"; }
pin_bare="$tmp/pin_bare"
mkdir -p "$pin_bare"

# The two files.
pin_version=$(sh "$pin_here/sdk-pin.sh")
expect "the record pin file has an exact SDK version" 0 sh -c "case '$pin_version' in [0-9]*.[0-9]*.[0-9]*) exit 0 ;; *) exit 1 ;; esac"
expect "the record pin does not roll forward" 0 sh -c "tr -d ' \n' <'$record_pin' | grep -q '\"rollForward\":\"disable\"'"
expect "the root global.json is a floor that rolls forward to a later feature band" 0 sh -c "tr -d ' \n' <'$repo_root/global.json' | grep -q '\"rollForward\":\"latestFeature\"'"
expect "the floor is the first 10.0 feature band" 0 sh -c "tr -d ' \n' <'$repo_root/global.json' | grep -q '\"version\":\"10.0.100\"'"
expect "the floor is not above the record pin" 0 sh -c "[ \"\$(printf '10.0.100\n%s\n' '$pin_version' | sort -V | head -1)\" = 10.0.100 ]"
expect "the fake dotnet: the floor accepts a later band (10.0.400)" 0 sh -c "[ \"\$(cd '$repo_root' && FAKE_NEWEST=10.0.400 '$pin_dotnet' --version)\" = 10.0.400 ]"
expect "the fake dotnet: the floor rejects a 9.x SDK" 145 sh -c "cd '$repo_root' && FAKE_NEWEST=9.0.300 '$pin_dotnet' --version"

# sdk-pin.sh prints the version of the sdk block of the file, and only that.
expect "sdk-pin.sh prints the version of the record pin file" 0 sh -c "[ \"\$(sh '$pin_here/sdk-pin.sh')\" = '$pin_version' ]"
sp="$tmp/sp"
mkdir -p "$sp"
printf '{"msbuild-sdks": {"Some.Sdk": "1.2.3", "version": "1"}, "sdk": {"version": "10.0.402", "rollForward": "disable"}}\n' >"$sp/msbuild-first.json"
expect "sdk-pin.sh ignores a version key outside the sdk block (msbuild-sdks first)" 0 sh -c "[ \"\$(SDK_PIN_FILE='$sp/msbuild-first.json' sh '$pin_here/sdk-pin.sh')\" = 10.0.402 ]"
printf '{"sdk": {"version": "10.0.402", "rollForward": "disable"}, "msbuild-sdks": {"version": "1"}}\n' >"$sp/msbuild-last.json"
expect "sdk-pin.sh ignores a version key after the sdk block (msbuild-sdks last)" 0 sh -c "[ \"\$(SDK_PIN_FILE='$sp/msbuild-last.json' sh '$pin_here/sdk-pin.sh')\" = 10.0.402 ]"
printf '{\n  "sdk": {\n    "rollForward": "disable",\n    "version":\n      "10.0.402"\n  }\n}\n' >"$sp/multiline.json"
expect "sdk-pin.sh reads a version on its own line" 0 sh -c "[ \"\$(SDK_PIN_FILE='$sp/multiline.json' sh '$pin_here/sdk-pin.sh')\" = 10.0.402 ]"
printf '{"msbuild-sdks": {"version": "1"}}\n' >"$sp/nosdk.json"
expect "sdk-pin.sh fails on a file with no sdk block" 1 env SDK_PIN_FILE="$sp/nosdk.json" sh "$pin_here/sdk-pin.sh"
printf '{"sdk": {"rollForward": "disable"}}\n' >"$sp/nover.json"
expect "sdk-pin.sh fails on an sdk block with no version" 1 env SDK_PIN_FILE="$sp/nover.json" sh "$pin_here/sdk-pin.sh"
printf '{"sdk": {"version": "latest"}}\n' >"$sp/badver.json"
expect "sdk-pin.sh fails on a version that is not major.minor.patch" 1 env SDK_PIN_FILE="$sp/badver.json" sh "$pin_here/sdk-pin.sh"
expect "sdk-pin.sh fails on a missing file" 1 env SDK_PIN_FILE="$sp/none.json" sh "$pin_here/sdk-pin.sh"
expect "sdk-pin.sh says what is wrong on stderr" 0 sh -c "[ -n \"\$(SDK_PIN_FILE='$sp/nover.json' sh '$pin_here/sdk-pin.sh' 2>&1 >/dev/null)\" ]"

# The failure seen in CI: the image carries 10.0.403 beside the installed 10.0.402. The record's own pin selects the pinned one,
# although the repository's floor would pick 10.0.403 (the working directory is the repository, as in CI).
pin_record "$repo_root" "10.0.402 10.0.403" "10.0.403"
expect "B1 passes when the image also carries a newer SDK (the record asks dotnet from its own pin)" 0 pin_has "ok   the SDK is the pinned $pin_version"
expect "the record shows the SDK it ran" 0 pin_has "dotnet SDK $pin_version"
expect "the SDK that builds the copies of the sources is the pinned one (the pin travels with them)" 1 pin_has "FAIL the SDK that builds the copy"
# B1 fails when dotnet reports something other than the pin.
pin_record "$repo_root" "10.0.402 10.0.403" "10.0.403" SDK_PINNED=10.0.403
expect "B1 fails when dotnet reports an SDK other than the pin" 0 pin_has "FAIL the SDK is not the pinned 10.0.403"
expect "B1 does not tick the SDK line when the SDK is not the pin" 1 pin_has "ok   the SDK is the pinned"
expect "the record exits 1 when the SDK is not the pin" 1 pin_record "$repo_root" "10.0.402 10.0.403" "10.0.403" SDK_PINNED=10.0.403
# Only a newer SDK installed: dotnet refuses (the record's pin does not roll forward) and the record fails.
pin_record "$repo_root" "10.0.403" "10.0.403"
expect "with only a newer SDK installed the record does not tick the SDK line" 1 pin_has "ok   the SDK is the pinned"
expect "with only a newer SDK installed the record exits 1" 1 pin_record "$repo_root" "10.0.403" "10.0.403"

# The probe record has the same exposure and the same fix.
PIN_SCRIPT=build-record-probe-dotnet.sh
pin_record "$repo_root" "10.0.402 10.0.403" "10.0.403"
expect "probe record: B1 passes when the image also carries a newer SDK" 0 pin_has "ok   the SDK is the pinned $pin_version"
expect "probe record: the SDK that builds the copies of the sources is the pinned one" 1 pin_has "FAIL the SDK that builds the copy"
pin_record "$repo_root" "10.0.402 10.0.403" "10.0.403" SDK_PINNED=10.0.403
expect "probe record: B1 fails when dotnet reports an SDK other than the pin" 0 pin_has "FAIL the SDK is not the pinned 10.0.403"
PIN_SCRIPT=build-record-dotnet.sh

# The docker wrapper resolves the pin first and stops when it cannot: it must not run docker with an empty SDK_PINNED.
fakebin="$tmp/pin_fakebin"
mkdir -p "$fakebin"
printf '#!/bin/sh\necho run >>"%s/docker.log"\nexit 0\n' "$tmp" >"$fakebin/docker"
chmod +x "$fakebin/docker"
rm -f "$tmp/docker.log"
expect "the docker wrapper stops when the pin cannot be read" 1 env PATH="$fakebin:$PATH" SDK_PIN_FILE="$sp/nover.json" SOURCE_COMMIT=test sh "$pin_here/build-record-docker.sh" dotnet-probe "$tmp/docker-out.md"
expect "the docker wrapper did not run docker with an empty pin" 1 test -e "$tmp/docker.log"
expect "the docker wrapper passes the pin from the record pin file" 0 grep -q 'SDK_PINNED="\$sdk_pinned"' "$pin_here/build-record-docker.sh"

# One source for the version (scripts/record-global.json); the literals that must repeat it are checked here.
ci_yml="$repo_root/.github/workflows/ci.yml"
expect "no record script hard-codes the pinned SDK as a default" 1 grep -nE 'SDK_PINNED:-[0-9]' "$here"/build-record-*.sh
expect "the docker wrapper's .NET image is the pinned SDK" 0 grep -q "dotnet/sdk:$pin_version@" "$here/build-record-docker.sh"
expect "the fixtures README names the pinned SDK" 0 grep -q "pinned $pin_version" "$here/../README.md"
expect "the .NET SDK README names the pinned SDK" 0 grep -q "$pin_version" "$repo_root/sdks/dotnet/README.md"
expect "ci.yml gives setup-dotnet no version of its own" 1 grep -nE '^ +dotnet-version:' "$ci_yml"
expect "every setup-dotnet step in ci.yml installs the SDK from the record pin file" 0 sh -c "n=\$(grep -c 'uses: actions/setup-dotnet' '$ci_yml'); [ \"\$n\" -ge 4 ] && [ \"\$(grep -c 'global-json-file: scripts/record-global.json' '$ci_yml')\" = \"\$n\" ]"

exit $failures
