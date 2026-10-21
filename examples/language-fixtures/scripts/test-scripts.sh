#!/bin/sh
# Tests of check-native.sh and run-checks.sh with a fake fixture program, so they need no compiler.
# The fake always reports an exact counter (as a perfect locked twin would), which makes it a racy program that
# never fires: the case that must be skipped on one CPU and rejected on several.

set -u
here=$(dirname "$0")
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at-test.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
failures=0

fake="$tmp/fake"
cat >"$fake" <<'EOF'
#!/bin/sh
echo 'HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"t.all_started","hits":1,"trues":1,"falses":0}' >&2
echo 'HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"t.counter_exact","hits":1,"trues":1,"falses":0}' >&2
echo 'lang=t   final=0000004000 order=0123456789abcdef rand=0123456789abcdef'
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

expect "a racy program that never fires is skipped on one CPU" 77 env LANG_AT_CPUS=1 sh "$here/check-native.sh" t "$fake" "$fake" 3
expect "a racy program that never fires fails on several CPUs" 1 env LANG_AT_CPUS=4 sh "$here/check-native.sh" t "$fake" "$fake" 3
expect "run-checks treats the skip as success on one CPU" 0 env LANG_AT_CPUS=1 sh "$here/run-checks.sh" t "$fake" "$fake" 3
expect "run-checks fails on several CPUs" 1 env LANG_AT_CPUS=4 sh "$here/run-checks.sh" t "$fake" "$fake" 3
expect "a wrong lang id is a contract failure, not a skip" 1 env LANG_AT_CPUS=1 sh "$here/check-native.sh" other "$fake" "$fake" 3

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
out=$(CHECK_NATIVE="$stub" sh "$here/run-checks.sh" t a b 1 2>&1)
rc=$?
if [ "$rc" -eq 0 ] && echo "$out" | grep -q "skip" && ! echo "$out" | grep -q "rejects"; then
  echo "ok   a skipped negative control is reported as skipped, not as a rejection"
else
  echo "FAIL a skipped negative control: exit $rc, output: $out"
  failures=$((failures + 1))
fi

exit $failures
