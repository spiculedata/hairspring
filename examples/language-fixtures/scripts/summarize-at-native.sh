#!/bin/sh
# The one-line summary of a native self-check log (make at-native), without changing the self-check's own output.
#
#   summarize-at-native.sh <log|-> <k>
#
# Reads the line `<lang> buggy: property false in <fired> of <runs> native runs` that check-native.sh prints and
# prints `runs=<runs> fired=<fired> k=<k>`. k is the iterations per worker the run used (a number, 1 or more).
# Exit 0 on success, 1 when the log has no such line, 64 for a usage error.

set -u

if [ $# -ne 2 ]; then
  echo "usage: $0 <log|-> <k>" >&2
  exit 64
fi
case "$2" in
  "" | *[!0-9]* | 0) echo "usage: k must be a positive number, got \"$2\"" >&2; exit 64 ;;
esac
if [ "$1" = - ]; then
  log=$(cat)
else
  log=$(cat "$1") || exit 1
fi
line=$(printf '%s\n' "$log" | sed -n 's/^[^ ]* buggy: property false in \([0-9][0-9]*\) of \([0-9][0-9]*\) native runs.*$/runs=\2 fired=\1/p' | head -n 1)
if [ -z "$line" ]; then
  echo "FAIL no 'buggy: property false in <n> of <n> native runs' line in the log" >&2
  exit 1
fi
echo "$line k=$2"
