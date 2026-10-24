#!/bin/sh
# Prints the environment variables of one profile arm of the .NET probe, one NAME=value per line, from profiles.manifest.
#
#   profile-env.sh <a|b> <dotnet-root> [manifest]
#
# Arm a prints the `a` lines; arm b prints the `a` lines and then the `b` lines. `@DOTNET_ROOT@` becomes <dotnet-root>.
# Comment lines and blank lines are skipped. A line that is not `<arm> <NAME=value>` is an error, so a typo in the manifest
# cannot silently drop a variable.
# Exits 0 on success, 1 for a bad manifest, 64 for a usage error.

set -u

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: $0 <a|b> <dotnet-root> [manifest]" >&2
  exit 64
fi
arm=$1
root=$2
manifest=${3:-$(dirname "$0")/profiles.manifest}
case "$arm" in
  a | b) ;;
  *)
    echo "usage: $0 <a|b> <dotnet-root> [manifest]: the arm is a or b, not '$arm'" >&2
    exit 64
    ;;
esac
if [ ! -r "$manifest" ]; then
  echo "FAIL $manifest cannot be read" >&2
  exit 1
fi

# Every entry must be well formed, whichever arm is asked for.
bad=$(grep -Ev '^(#|[[:space:]]*$|[ab] [A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*$)' "$manifest")
if [ -n "$bad" ]; then
  echo "FAIL $manifest has a line that is not '<arm> <NAME=value>': $bad" >&2
  exit 1
fi

emit() { # emit <arm>: the variables of one arm, with the runtime directory filled in
  grep -E "^$1 " "$manifest" | cut -d' ' -f2- | while IFS= read -r entry; do
    printf '%s\n' "$entry" | sed "s|@DOTNET_ROOT@|$root|g"
  done
}

emit a
[ "$arm" = b ] && emit b
exit 0
