#!/bin/sh
# Extracts a pinned .NET runtime tarball into a new directory, after checking its sha512.
#
#   extract-runtime.sh <tarball> <sha512> <destination>
#
# The build record runs the probe on exactly this runtime (DOTNET_ROOT is the destination), so it cannot silently run on
# whichever 10.0.x patch the machine has installed. The tarball is hashed first and nothing is extracted when the sha512 is
# not the given 128 hex digits. The destination must not exist; it is created. Prints the destination on success.
# Exits 0 on success, 1 when the tarball is missing, the hash differs, the destination exists or the extraction fails, 64
# for a usage error.

set -u

if [ $# -ne 3 ]; then
  echo "usage: $0 <tarball> <sha512> <destination>" >&2
  exit 64
fi
tarball=$1
want=$2
dest=$3
if [ ! -r "$tarball" ]; then
  echo "FAIL $tarball cannot be read" >&2
  exit 1
fi
if ! printf '%s' "$want" | grep -Eq '^[0-9a-f]{128}$'; then
  echo "FAIL the sha512 must be 128 lowercase hex digits" >&2
  exit 1
fi
if [ -e "$dest" ]; then
  echo "FAIL $dest exists" >&2
  exit 1
fi
if command -v sha512sum >/dev/null 2>&1; then
  got=$(sha512sum "$tarball" | cut -d' ' -f1)
else
  got=$(shasum -a 512 "$tarball" | cut -d' ' -f1)
fi
echo "sha512 of $(basename "$tarball"): $got"
if [ "$got" != "$want" ]; then
  echo "FAIL the sha512 is not the pinned $want" >&2
  exit 1
fi
mkdir -p "$dest" || exit 1
if ! tar -xzf "$tarball" -C "$dest"; then
  echo "FAIL the extraction failed" >&2
  exit 1
fi
echo "ok   extracted into $dest"
