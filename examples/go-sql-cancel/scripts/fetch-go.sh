#!/bin/sh
# Download the official Go archives this example needs into .cache/dl and verify each against PINS.
#
#   scripts/fetch-go.sh           download what is missing, verify everything
#   scripts/fetch-go.sh --live    also compare every PINS entry with go.dev's live JSON listing (needs jq)
#
# Needs curl and shasum. Nothing is installed; archives are only unpacked by the other scripts.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
dl="$here/.cache/dl"
base="https://go.dev/dl"
# Archives downloaded here. The linux-amd64 archives are pinned for readers on Linux but not needed on this host.
files="go1.24.5.darwin-arm64.tar.gz go1.24.6.darwin-arm64.tar.gz go1.24.5.src.tar.gz"

mkdir -p "$dl"

pin_for() {
  awk -v f="$1" '$2 == f { print $1 }' "$here/PINS"
}

for f in $files; do
  if [ -f "$dl/$f" ]; then continue; fi
  echo "fetch $f"
  curl -fsSL -o "$dl/$f.part" "$base/$f"
  mv "$dl/$f.part" "$dl/$f"
done

status=0
for f in $files; do
  want=$(pin_for "$f")
  if [ -z "$want" ]; then
    echo "FAIL $f has no entry in PINS"
    status=1
    continue
  fi
  got=$(shasum -a 256 "$dl/$f" | awk '{ print $1 }')
  if [ "$got" = "$want" ]; then
    echo "ok   $f $got"
  else
    echo "FAIL $f got $got want $want"
    status=1
  fi
done

if [ "${1:-}" = "--live" ]; then
  listing=$(curl -fsSL "$base/?mode=json&include=all")
  for f in $(awk '!/^#/ && NF == 2 { print $2 }' "$here/PINS"); do
    live=$(printf '%s' "$listing" | jq -r --arg f "$f" '[.[].files[] | select(.filename == $f) | .sha256][0] // empty')
    want=$(pin_for "$f")
    if [ "$live" = "$want" ]; then
      echo "ok   live listing agrees: $f"
    else
      echo "FAIL live listing for $f is '$live', PINS has '$want'"
      status=1
    fi
  done
fi

exit "$status"
