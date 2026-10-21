#!/bin/sh
# Source check of a lost-update fixture: it must not read the scheduler seed, /proc, the CPU count or the clock.
#
#   check-sources.sh <source-file-or-directory>...
#
# The fixture contract forbids these inputs outside the language runtime and the stated random source: the seed, /proc,
# the CPU count (sysconf, nproc, NumCPU, sched_getaffinity, get_nprocs, hardware_concurrency, runtime.GOMAXPROCS,
# available_parallelism), the clock (clock_gettime, clock(), timespec_get, gettimeofday, time(), std::chrono, time.Now,
# time.Since, Instant::now, SystemTime, rdtsc) and the environment (getenv, os.Getenv, os.LookupEnv, os.Environ, env::var).
# Any of them would make a run depend on something the deterministic scheduler does not control. The Go program does not
# need runtime.GOMAXPROCS: the engine sets GOMAXPROCS in the environment. This is a plain text search, so a forbidden
# word in a comment is reported too; keep the sources free of them.
#
# A directory is searched whole (every file below it except Go test files and bytecode), so a new source file is covered
# without editing a list. A file that is missing or unreadable, a grep that fails, or nothing to search is a failure, never
# a pass. On success it prints the pattern and the files, so a build record shows what was searched and that nothing
# matched.
# Exits 0 when every file is clean, 1 when a forbidden token is found or a file cannot be read, 64 for a usage error.

set -u

if [ $# -lt 1 ]; then
  echo "usage: $0 <source-file-or-directory>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|sysconf|nproc|NumCPU|hardware_concurrency|clock_gettime|gettimeofday|time\.Now|time\.Since|getenv|Getenv|LookupEnv|Environ\(|sched_getaffinity|get_nprocs|std::chrono|[^A-Za-z0-9_.]time\(|[^A-Za-z0-9_.]clock\(|timespec_get|rdtsc|runtime\.GOMAXPROCS|available_parallelism|Instant::now|SystemTime|env::var'
list=$(mktemp "${TMPDIR:-/tmp}/sources.XXXXXX") || exit 1
trap 'rm -f "$list"' EXIT
status=0
for arg in "$@"; do
  if [ -d "$arg" ]; then
    find "$arg" -type f ! -name '*_test.go' ! -name '*.pyc' | LC_ALL=C sort >>"$list"
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
while read -r file; do
  grep -En "$forbidden" "$file" >&2
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
  echo "grep -En '$forbidden'"
  echo "no match in: $(tr '\n' ' ' <"$list")"
fi
exit $status
