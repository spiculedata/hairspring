#!/bin/sh
# The self-check of one language plus its negative control, for the three Makefiles.
#
#   run-checks.sh <lang-id> <racy-binary> <locked-binary> [runs]
#
# 1. check-native.sh on the real pair. Exit 0 continues; exit 77 (the racy check was skipped on a single-CPU host) ends
#    here with a notice and success, because the negative control cannot work there either; anything else fails.
# 2. The negative control: the racy program stands in for the locked twin and check-native.sh must reject it, or the
#    check would pass whatever it was given. If that run reports "skipped" (77) the control did not run: that is
#    reported as a skip, never as a rejection.
#
# CHECK_NATIVE names the checker (default: check-native.sh beside this script); the script tests use it.

set -u

if [ $# -lt 3 ]; then
  echo "usage: $0 <lang-id> <racy-binary> <locked-binary> [runs]" >&2
  exit 64
fi
here=$(dirname "$0")
check=${CHECK_NATIVE:-$here/check-native.sh}
lang=$1
racy=$2
locked=$3
runs=${4:-50}
SKIPPED=77

sh "$check" "$lang" "$racy" "$locked" "$runs"
rc=$?
if [ "$rc" -eq "$SKIPPED" ]; then
  echo "skip: $lang self-check skipped (see above); the negative control is skipped with it"
  exit 0
fi
if [ "$rc" -ne 0 ]; then
  exit "$rc"
fi

sh "$check" "$lang" "$racy" "$racy" "$runs" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq "$SKIPPED" ]; then
  echo "skip: the $lang negative control was skipped; it did not show that the check can fail"
  exit 0
fi
if [ "$rc" -eq 0 ]; then
  echo "FAIL the self-check accepted the racy program as a locked twin" >&2
  exit 1
fi
echo "ok   the self-check rejects a racy program standing in for the locked twin"
