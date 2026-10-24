#!/bin/sh
# The native self-check of the Java fixture (run-checks.sh: the contract, 50 runs, a negative control) once for each JVM flag
# profile, with that profile's flags in JAVA_FLAGS (the at-java wrapper reads them when it runs).
#
#   run-java-profile-checks.sh <profiles.tsv> <artifact> [runs]
#
# RUN_CHECKS names the checker (default: run-checks.sh beside this script); the script tests use it. Every profile is run
# even after one fails. Exits 0 when every profile passed, 1 when one failed or there is none, 64 for a usage error.

set -u

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <profiles.tsv> <artifact> [runs]" >&2
  exit 64
fi
here=$(cd "$(dirname "$0")" && pwd)
profiles=$1
artifact=$2
runs=${3:-50}
checker=${RUN_CHECKS:-$here/run-checks.sh}
if [ ! -r "$profiles" ]; then
  echo "FAIL $profiles cannot be read" >&2
  exit 1
fi
tab=$(printf '\t')
status=0
count=0
while IFS="$tab" read -r arm flags; do
  case "$arm" in '' | '#'*) continue ;; esac
  count=$((count + 1))
  echo "--- arm $arm: $flags"
  if JAVA_FLAGS=$flags sh "$checker" java "$artifact" "$runs"; then
    echo "ok   arm $arm: the self-check passed under these flags"
  else
    echo "FAIL arm $arm: the self-check failed under these flags"
    status=1
  fi
done <"$profiles"
if [ "$count" -eq 0 ]; then
  echo "FAIL $profiles holds no profile" >&2
  status=1
fi
exit $status
