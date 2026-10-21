#!/bin/sh
# Checks that an executable is statically linked, from the text `file <binary>` printed (read on stdin).
#
#   file "$binary" | check-static.sh
#
# Accepts "statically linked" and "static-pie linked". Empty input (for example, no `file` command) is a failure, never a
# pass: the build record must assert the property, not skip it.
# Exits 0 when the text says static, 1 when not, 64 for a usage error.

set -u

if [ $# -ne 0 ]; then
  echo "usage: file <binary> | $0" >&2
  exit 64
fi
text=$(cat)
if [ -z "$text" ]; then
  echo "FAIL no output from file: cannot tell whether the binary is static" >&2
  exit 1
fi
echo "$text"
if printf '%s\n' "$text" | grep -Eq 'statically linked|static-pie linked'; then
  echo "ok   the binary is statically linked"
else
  echo "FAIL the binary is not statically linked" >&2
  exit 1
fi
