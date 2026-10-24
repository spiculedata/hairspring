#!/bin/sh
# Checks that a captured stdout is exactly one Java probe line and nothing else.
#
#   check-probe-line.sh <stdout-file>
#
# The line must match the six-field form and end in a newline, and the file must be exactly that line: its size in bytes is
# the first line's length including the newline. So a second line, a blank line, trailing spaces and junk after the line
# (with or without a newline) are all rejected; counting newlines alone would accept `LINE\njunk`.
# Exits 0 when the file is exactly the line, 1 when not, 64 for a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <stdout-file>" >&2
  exit 64
fi
file=$1
if [ ! -r "$file" ]; then
  echo "FAIL $file cannot be read" >&2
  exit 1
fi
line_rx='^PROBE v1 lang=java lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'
total=$(wc -c <"$file" | tr -d ' ')
first=$(head -n 1 "$file" | wc -c | tr -d ' ')
if [ "$total" -eq 0 ]; then
  echo "FAIL stdout is empty" >&2
  exit 1
fi
if [ "$total" -ne "$first" ]; then
  echo "FAIL stdout holds $total bytes but its first line is $first bytes: something follows the line" >&2
  exit 1
fi
if [ "$(tail -c 1 "$file" | wc -l | tr -d ' ')" -ne 1 ]; then
  echo "FAIL the line does not end in a newline" >&2
  exit 1
fi
if ! head -n 1 "$file" | grep -Eq "$line_rx"; then
  echo "FAIL not the six-field line:" >&2
  head -n 1 "$file" >&2
  exit 1
fi
exit 0
