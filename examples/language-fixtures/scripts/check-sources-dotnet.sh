#!/bin/sh
# Source check of the .NET probe: it must not read the scheduler seed, /proc, the CPU count, the environment or the clock.
#
#   check-sources-dotnet.sh [--allow-stopwatch] <source-file-or-directory>...
#
# Forbidden: HAIRSPRING_SEED, /proc, Environment.GetEnvironmentVariable(s), ExpandEnvironmentVariables, the CPU count
# (Environment.ProcessorCount, Thread.GetCurrentProcessorId), the wall clock and tick counters (DateTime.Now, UtcNow, Today,
# DateTimeOffset.Now, UtcNow, Environment.TickCount and TickCount64, TimeProvider), Process.GetCurrentProcess and Stopwatch.
# --allow-stopwatch is for the probe alone: its output line carries the monotonic counter difference between the start and
# the end of the run, so `Stopwatch.GetTimestamp(` and `Stopwatch.Frequency` are allowed there and no other use of Stopwatch
# is (not StartNew, not GetElapsedTime, not `new Stopwatch`). check-stopwatch-use.sh is the other half: the file that holds
# the read has exactly one. This is a plain text search, so a forbidden word in a comment is reported too.
#
# A directory is searched whole (every .cs file below it except obj/ and bin/ build output), so a new source file is covered
# without editing a list. A file that is missing or unreadable, a grep that fails, or nothing to search is a failure, never
# a pass. On success it prints the pattern and the files, so a build record shows what was searched and that nothing matched.
# Exits 0 when every file is clean, 1 when a forbidden token is found or a file cannot be read, 64 for a usage error.

set -u

allow_stopwatch=0
if [ "${1:-}" = "--allow-stopwatch" ]; then
  allow_stopwatch=1
  shift
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--allow-stopwatch] <source-file-or-directory>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|GetEnvironmentVariable|ExpandEnvironmentVariables|ProcessorCount|ProcessorId|DateTime\.(Now|UtcNow|Today)|DateTimeOffset\.(Now|UtcNow)|TickCount|TimeProvider|GetCurrentProcess|Stopwatch'
list=$(mktemp "${TMPDIR:-/tmp}/sources.XXXXXX") || exit 1
trap 'rm -f "$list"' EXIT
status=0
for arg in "$@"; do
  if [ -d "$arg" ]; then
    find "$arg" -type f -name '*.cs' ! -path '*/obj/*' ! -path '*/bin/*' | LC_ALL=C sort >>"$list"
  elif [ -r "$arg" ]; then
    printf '%s\n' "$arg" >>"$list"
  else
    echo "FAIL $arg is missing or unreadable" >&2
    status=1
  fi
done
if [ ! -s "$list" ]; then
  echo "FAIL no source files to search" >&2
  status=1
fi

# without_allowed <file>: the file with the allowed stopwatch forms blanked out (line numbers are kept), or as it is.
without_allowed() {
  if [ "$allow_stopwatch" -eq 1 ]; then
    sed -E 's/Stopwatch\.GetTimestamp\(|Stopwatch\.Frequency//g' "$1"
  else
    cat "$1"
  fi
}

while read -r file; do
  without_allowed "$file" | grep -En "$forbidden" >&2
  rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "FAIL $file reads the seed, /proc, the CPU count, the clock or the environment" >&2
    status=1
  elif [ "$rc" -ne 1 ]; then
    echo "FAIL $file could not be searched (grep exit status $rc)" >&2
    status=1
  fi
done <"$list"
if [ "$status" -eq 0 ]; then
  note=""
  [ "$allow_stopwatch" -eq 1 ] && note=" (Stopwatch.GetTimestamp( and Stopwatch.Frequency blanked out first)"
  echo "grep -En '$forbidden'$note"
  echo "no match in: $(tr '\n' ' ' <"$list")"
fi
exit $status
