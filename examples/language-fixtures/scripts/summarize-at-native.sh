#!/bin/sh
# The one-line summary of a native self-check log (make at-native), without changing the self-check's own output.
#
#   summarize-at-native.sh <log|-> <k>
#
# Reads the line `<lang> buggy: property false in <fired> of <runs> native runs` that check-native.sh prints and
# prints `runs=<runs> fired=<fired> k=<k>`. k is the iterations per worker the run used: 1 to 5000, no leading zero.
# The log must hold exactly one `<lang> buggy:` line, and it must match whole (no trailing text); runs is 1 or more and
# fired is at most runs. Exit 0 on success, 1 when the log does not give a valid summary, 64 for a usage error.

set -u

MAX_K=5000
if [ $# -ne 2 ]; then
  echo "usage: $0 <log|-> <k>" >&2
  exit 64
fi
case "$2" in
  "" | *[!0-9]* | 0* | ?????*) echo "usage: k must be a number from 1 to $MAX_K without a leading zero, got \"$2\"" >&2; exit 64 ;;
esac
if [ "$2" -gt "$MAX_K" ]; then
  echo "usage: k must be a number from 1 to $MAX_K without a leading zero, got \"$2\"" >&2
  exit 64
fi
if [ "$1" = - ]; then
  log=$(cat)
else
  log=$(cat "$1") || exit 1
fi
if [ "$(printf '%s\n' "$log" | grep -c '^[^ ]* buggy:')" -ne 1 ]; then
  echo "FAIL the log must hold exactly one '<lang> buggy:' line" >&2
  exit 1
fi
line=$(printf '%s\n' "$log" | sed -n 's/^[^ ]* buggy: property false in \([0-9][0-9]*\) of \([0-9][0-9]*\) native runs$/\1 \2/p' | head -n 1)
if [ -z "$line" ]; then
  echo "FAIL the 'buggy:' line is not 'property false in <n> of <n> native runs'" >&2
  exit 1
fi
fired=${line% *}
runs=${line#* }
if [ "${#fired}" -gt 6 ] || [ "${#runs}" -gt 6 ] || [ "$runs" -lt 1 ] || [ "$fired" -gt "$runs" ]; then
  echo "FAIL fired=$fired runs=$runs is not a possible result" >&2
  exit 1
fi
echo "runs=$runs fired=$fired k=$2"
