#!/bin/sh
# The one-line summary of a native self-check log (make at-native and the profile targets), without changing the
# self-check's own output.
#
#   summarize-at-native.sh <log|-> <k> [profile]
#
# Reads the line `<lang> buggy: property false in <fired> of <runs> native runs` that check-native.sh prints and
# prints `runs=<runs> fired=<fired> k=<k>`. k is the iterations per worker the run used: 1 to 5000, no leading zero.
# The log must hold exactly one `<lang> buggy:` line, and it must match whole (no trailing text); runs is 1 or more and
# fired is at most runs.
#
# With a profile (letters, digits and dashes, not starting with a dash) the log is the output of
# run-java-profile-checks.sh: one section per JVM flag profile, each opened by `--- arm <profile>: <flags>`. The log must
# hold exactly one section of that profile, which must hold exactly one `buggy:` line; the other profiles' lines are
# ignored, and the summary names the profile: `profile=<profile> runs=<runs> fired=<fired> k=<k>`.
#
# Exit 0 on success, 1 when the log does not give a valid summary, 64 for a usage error.

set -u

MAX_K=5000
if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <log|-> <k> [profile]" >&2
  exit 64
fi
case "$2" in
  "" | *[!0-9]* | 0* | ?????*) echo "usage: k must be a number from 1 to $MAX_K without a leading zero, got \"$2\"" >&2; exit 64 ;;
esac
if [ "$2" -gt "$MAX_K" ]; then
  echo "usage: k must be a number from 1 to $MAX_K without a leading zero, got \"$2\"" >&2
  exit 64
fi
profile=${3-}
if [ $# -eq 3 ]; then
  case "$profile" in
    "" | -* | *[!A-Za-z0-9-]*) echo "usage: profile must be letters, digits and dashes, not starting with a dash, got \"$profile\"" >&2; exit 64 ;;
  esac
fi
if [ "$1" = - ]; then
  log=$(cat)
else
  log=$(cat "$1") || exit 1
fi
if [ $# -eq 3 ]; then
  # Only the lines of the one section of this profile: from its `--- arm <profile>:` line to the next `--- arm` line.
  sections=$(printf '%s\n' "$log" | awk -v p="--- arm $profile:" 'index($0, p) == 1 { n++ } END { print n + 0 }')
  if [ "$sections" -ne 1 ]; then
    echo "FAIL the log must hold exactly one section of profile $profile, found $sections" >&2
    exit 1
  fi
  log=$(printf '%s\n' "$log" | awk -v p="--- arm $profile:" 'index($0, "--- arm ") == 1 { on = (index($0, p) == 1) } on')
fi
if [ "$(printf '%s\n' "$log" | grep -c '^[^ ]* buggy:')" -ne 1 ]; then
  echo "FAIL the log must hold exactly one '<lang> buggy:' line${profile:+ for profile $profile}" >&2
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
echo "${3+profile=$profile }runs=$runs fired=$fired k=$2"
