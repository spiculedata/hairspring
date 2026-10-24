#!/bin/sh
# Source check of the .NET probe: it must not read the scheduler seed, /proc, the CPU count, the environment or the clock.
#
#   check-sources-dotnet.sh [--allow-stopwatch-in <file-name>] <source-file-or-directory>...
#
# Forbidden: HAIRSPRING_SEED, /proc and /sys (CPU counts and cgroup limits are read there), Environment.GetEnvironmentVariable(s),
# ExpandEnvironmentVariables, the CPU count (Environment.ProcessorCount, Thread.GetCurrentProcessorId), the wall clock and tick
# counters (DateTime.Now, UtcNow, Today, DateTimeOffset.Now, UtcNow, a bare Now, UtcNow or Today, Environment.TickCount and
# TickCount64, TimeProvider), `using static` of DateTime, DateTimeOffset or Environment, Process.GetCurrentProcess and Stopwatch.
# --allow-stopwatch-in is for the probe alone: its output line carries the monotonic counter difference between the start and
# the end of the run, so in the one source file with that name (matched on the file name) `Stopwatch.GetTimestamp(` and
# `Stopwatch.Frequency` are allowed and no other use of Stopwatch is (not StartNew, not GetElapsedTime, not `new Stopwatch`);
# every other file is searched without the allowance, so a second clock read elsewhere fails. check-stopwatch-use.sh is the
# other half: the file that holds the read has exactly one. This is a plain text search, so a forbidden word in a comment is
# reported too.
#
# A directory is searched whole (every .cs file below it except obj/ and bin/ build output), so a new source file is covered
# without editing a list. A file that is missing or unreadable, a grep that fails, or nothing to search is a failure, never
# a pass. On success it prints the pattern and the files, so a build record shows what was searched and that nothing matched.
# Exits 0 when every file is clean, 1 when a forbidden token is found or a file cannot be read, 64 for a usage error.

set -u

allowed_name=""
if [ "${1:-}" = "--allow-stopwatch-in" ]; then
  if [ $# -lt 2 ]; then
    echo "usage: $0 [--allow-stopwatch-in <file-name>] <source-file-or-directory>..." >&2
    exit 64
  fi
  allowed_name=$2
  shift 2
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--allow-stopwatch-in <file-name>] <source-file-or-directory>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|/sys/|GetEnvironmentVariable|ExpandEnvironmentVariables|ProcessorCount|ProcessorId|DateTime\.(Now|UtcNow|Today)|DateTimeOffset\.(Now|UtcNow)|(^|[^A-Za-z0-9_.])(Now|UtcNow|Today)([^A-Za-z0-9_]|$)|TickCount|TimeProvider|using[[:space:]]+static[[:space:]]+System\.(DateTime|DateTimeOffset|Environment)|GetCurrentProcess|Stopwatch'
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

# without_allowed <file>: the named file with the allowed stopwatch forms blanked out (line numbers are kept); any other
# file as it is.
without_allowed() {
  if [ -n "$allowed_name" ] && [ "$(basename "$1")" = "$allowed_name" ]; then
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
  [ -n "$allowed_name" ] && note=" (in $allowed_name only: Stopwatch.GetTimestamp( and Stopwatch.Frequency blanked out first)"
  echo "grep -En '$forbidden'$note"
  echo "no match in: $(tr '\n' ' ' <"$list")"
fi
exit $status
