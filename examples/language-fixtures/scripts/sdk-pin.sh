#!/bin/sh
# Prints the .NET SDK version pinned for the build records: the "sdk" block of scripts/record-global.json, the one place the
# version is written (the root global.json is only a floor for contributors).
#
#   sdk-pin.sh
#
# The record scripts use it as the default of SDK_PINNED and copy the file as global.json into the directories they build
# from. SDK_PIN_FILE names another file (for the tests). Exits 1, saying why on stderr, if the file is missing or has no
# "sdk" block with an exact major.minor.patch version.
here=$(cd "$(dirname "$0")" && pwd)
file=${SDK_PIN_FILE:-$here/../../../scripts/record-global.json}
[ -f "$file" ] || { echo "sdk-pin.sh: $file not found" >&2; exit 1; }
# The file on one line, then the body of the "sdk" object (a key of exactly that name, so "msbuild-sdks" does not match).
block=$(tr -d '\n' <"$file" | sed -n 's/.*"sdk"[[:space:]]*:[[:space:]]*{\([^}]*\)}.*/\1/p')
[ -n "$block" ] || { echo "sdk-pin.sh: no \"sdk\" block in $file" >&2; exit 1; }
version=$(printf '%s' "$block" | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
[ -n "$version" ] || { echo "sdk-pin.sh: no \"version\" in the \"sdk\" block of $file" >&2; exit 1; }
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "sdk-pin.sh: \"$version\" in $file is not an exact major.minor.patch version" >&2; exit 1 ;;
esac
printf '%s\n' "$version"
