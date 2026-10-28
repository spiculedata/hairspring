#!/bin/sh
# Prints the .NET SDK version pinned in the repository's global.json (the one place the pin is written down).
#
#   sdk-pin.sh
#
# The record scripts use it as the default of SDK_PINNED, and the manual CI job installs exactly this version. Exits 1 if
# global.json is missing or has no sdk version.
here=$(cd "$(dirname "$0")" && pwd)
file=$here/../../../global.json
[ -f "$file" ] || { echo "sdk-pin.sh: $file not found" >&2; exit 1; }
version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$file" | head -1)
[ -n "$version" ] || { echo "sdk-pin.sh: no sdk version in $file" >&2; exit 1; }
printf '%s\n' "$version"
