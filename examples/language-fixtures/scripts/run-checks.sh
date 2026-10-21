#!/bin/sh
# The self-check of one language plus its negative control, for the three Makefiles.
#
#   run-checks.sh <lang-id> <artifact> [runs]
#
# 1. check-native.sh on the real artifact. Exit 0 continues; exit 77 (the buggy check was skipped on a single-CPU
#    host) ends here with a notice and success, because the negative control cannot work there either; anything else
#    fails.
# 2. The negative control: a stand-in that runs the artifact's buggy mode whenever it is asked for fixed, so the fixed
#    mode loses updates. check-native.sh must reject it, or the check would pass whatever it was given. If that run
#    reports "skipped" (77) the control did not run: that is reported as a skip, never as a rejection.
#
# CHECK_NATIVE names the checker (default: check-native.sh beside this script); the script tests use it.

set -u

if [ $# -lt 2 ]; then
  echo "usage: $0 <lang-id> <artifact> [runs]" >&2
  exit 64
fi
here=$(dirname "$0")
check=${CHECK_NATIVE:-$here/check-native.sh}
lang=$1
artifact=$2
runs=${3:-50}
SKIPPED=77

sh "$check" "$lang" "$artifact" "$runs"
rc=$?
if [ "$rc" -eq "$SKIPPED" ]; then
  echo "skip: $lang self-check skipped (see above); the negative control is skipped with it"
  exit 0
fi
if [ "$rc" -ne 0 ]; then
  exit "$rc"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lang-at-control.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
control="$tmp/control"
# Only the fixed mode is rewritten, so the argument checks and the other modes still see the real artifact. The line
# says mode=fixed, so the only thing wrong with the stand-in is that it loses updates.
cat >"$control" <<EOF
#!/bin/sh
if [ "\$#" -eq 1 ] && [ "\$1" = fixed ]; then
  "$artifact" buggy | sed 's/ mode=buggy / mode=fixed /'
  exit 0
fi
exec "$artifact" "\$@"
EOF
chmod +x "$control"

sh "$check" "$lang" "$control" "$runs" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq "$SKIPPED" ]; then
  echo "skip: the $lang negative control was skipped; it did not show that the check can fail"
  exit 0
fi
if [ "$rc" -eq 0 ]; then
  echo "FAIL the self-check accepted a fixed mode that is really the buggy one" >&2
  exit 1
fi
echo "ok   the self-check rejects a fixed mode that is really the buggy one"
