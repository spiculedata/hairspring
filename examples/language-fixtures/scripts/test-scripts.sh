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
  fieldfalses="\"falses\":   $2"
  [ "${FAKE:-}" = nofalses ] && fieldfalses="\"falses\":null"
  echo "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"$1\",\"hits\":   $4,\"trues\":   $trues,$fieldfalses,\"first_false\":null,\"first_true\":{\"thread\":\"t#1\",\"ord\":1,\"vals\":$3}}" >&2
}
event "$id.all_started" 0 "[4,0,0,0]" "$hits"
event "$id.counter_exact" "$falses" "$vals" "$hits"
[ "${FAKE:-}" = incomplete ] && echo "hairspring: incomplete catalog: 1 evaluation(s) ran at call sites that are missing from the catalog section" >&2

[ "${FAKE:-}" = dupe ] && event "$id.counter_exact" 1 "[3990,4000,4,1000]" 1
if ! printf "%s lang=%s mode=%s final=%0${digits}d expect=%020d order=0123456789abcdef rand=0123456789abcdef\n" "$head" "$lang" "$shown" "$final" 4000 2>/dev/null; then
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
  getenv os.Getenv sched_getaffinity get_nprocs 'std::chrono' ' time(0)' rdtsc runtime.GOMAXPROCS; do
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

exit $failures
