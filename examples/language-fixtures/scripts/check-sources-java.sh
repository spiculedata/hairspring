#!/bin/sh
# Source check of the Java probe: it must not read the scheduler seed, /proc, the environment, the CPU count, the process
# table or the clock.
#
#   check-sources-java.sh [--allow-nano-time] <source-file-or-directory>...
#
# The contract forbids these inputs outside the language runtime and the stated random source: the seed (HAIRSPRING_SEED),
# /proc, the environment (System.getenv), the CPU count (availableProcessors), the process table (ProcessHandle,
# ProcessBuilder), management beans, system properties, files under /sys/, reflection, other random sources
# (ThreadLocalRandom, Math.random, SecureRandom, UUID.randomUUID, SplittableRandom, RandomGenerator.getDefault,
# RandomGeneratorFactory), running programs (ProcessBuilder, Runtime.exec) and the clock (System.currentTimeMillis, Instant.now,
# LocalDateTime.now, OffsetDateTime, InstantSource, Calendar, Clock, Date, Year.now).
# `--allow-nano-time` is for the probe alone: its output line carries the monotonic clock difference between the start and
# the end of the run (vtime_ns), so System.nanoTime is allowed there, exactly twice (the start and the end), and nothing
# else of the clock is. A static import of System, or any other way to name nanoTime without `System.`, is rejected, so a
# third read cannot hide. This is a plain text search, so a forbidden word in a comment is reported too; keep the sources
# free of them.
#
# A directory is searched whole (every .java file below it). A file that is missing or unreadable, a grep that fails, or
# nothing to search is a failure, never a pass. On success it prints the pattern and the files.
# Exits 0 when every file is clean, 1 when a forbidden token is found or a file cannot be read, 64 for a usage error.

set -u

allow_nano=0
if [ "${1:-}" = "--allow-nano-time" ]; then
  allow_nano=1
  shift
fi
if [ $# -lt 1 ]; then
  echo "usage: $0 [--allow-nano-time] <source-file-or-directory>..." >&2
  exit 64
fi

forbidden='HAIRSPRING_SEED|/proc|getenv|availableProcessors|ProcessHandle|ProcessBuilder|currentTimeMillis|Instant\.now|LocalDateTime|ZonedDateTime|LocalDate\.now|LocalTime\.now|Clock\.|java\.util\.Date|new Date|ManagementFactory|import static java\.lang\.System|Calendar\.getInstance|OffsetDateTime|InstantSource|Year\.now|YearMonth\.now|MonthDay\.now|System\.getProperty|System\.getProperties|/sys/|Class\.forName|getDeclaredMethod|getDeclaredField|setAccessible|ThreadLocalRandom|Math\.random|SecureRandom|UUID\.randomUUID|SplittableRandom|RandomGenerator\.getDefault|RandomGeneratorFactory|getRuntime\(\)\.exec'
nano='nanoTime'
if [ "$allow_nano" -eq 0 ]; then
  forbidden="$forbidden|$nano"
fi
list=$(mktemp "${TMPDIR:-/tmp}/sources.XXXXXX") || exit 1
trap 'rm -f "$list"' EXIT
status=0
for arg in "$@"; do
  if [ -d "$arg" ]; then
    find "$arg" -type f -name '*.java' | LC_ALL=C sort >>"$list"
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
reads=0
named=0
while read -r file; do
  grep -En "$forbidden" "$file" >&2
  rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "FAIL $file reads the seed, /proc, the environment, the CPU count, the process table or the clock" >&2
    status=1
  elif [ "$rc" -ne 1 ]; then
    echo "FAIL $file could not be searched (grep exit status $rc)" >&2
    status=1
  fi
  reads=$((reads + $(grep -o 'System\.nanoTime' "$file" | wc -l)))
  named=$((named + $(grep -o "$nano" "$file" | wc -l)))
done <"$list"
if [ "$allow_nano" -eq 1 ]; then
  if [ "$named" -ne "$reads" ]; then
    echo "FAIL nanoTime is named $named times but read through System.nanoTime $reads times: a read hides behind another name" >&2
    status=1
  elif [ "$reads" -ne 2 ]; then
    echo "FAIL $reads System.nanoTime reads, want exactly 2 (the start and the end of the run)" >&2
    status=1
  fi
fi
if [ "$status" -eq 0 ]; then
  echo "grep -En '$forbidden'"
  [ "$allow_nano" -eq 0 ] || echo "System.nanoTime: $reads reads (the start and the end of the run)"
  echo "no match in: $(tr '\n' ' ' <"$list")"
fi
exit $status
