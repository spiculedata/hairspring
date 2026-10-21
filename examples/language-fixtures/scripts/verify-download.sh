#!/bin/sh
# Verifies an artifact as it was downloaded from a run against the build record that describes it, as a separate step from
# the build: it recomputes the sha256 from the downloaded bytes and compares it with SHA256SUMS and with the record.
#
#   verify-download.sh <downloaded-artifact-dir> <record-file>
#
# The directory is one `fixture-<variant>` artifact of the run, unzipped. It holds SHA256SUMS and either one binary or a
# package directory with a `<name>.tree` file (the tree hash: the sha256 of the sorted `<sha256>  <path>` lines of the
# package, paths relative to the package). The record's `Artifact:` line ends with the sha256 or tree hash it names.
# Checks: `sha256sum -c SHA256SUMS` on the downloaded bytes; for a package, the tree hash recomputed from the files equals
# the `.tree` file; the value equals the one in the record. A zip download loses the executable bit, which does not change
# the sha256.
# Exits 0 when everything matches, 1 when not, 64 for a usage error.

set -u

if [ $# -ne 2 ]; then
  echo "usage: $0 <downloaded-artifact-dir> <record-file>" >&2
  exit 64
fi
dir=$1
record=$2
if [ ! -d "$dir" ] || [ ! -r "$dir/SHA256SUMS" ]; then
  echo "FAIL $dir has no SHA256SUMS: not a downloaded artifact directory" >&2
  exit 1
fi
if [ ! -r "$record" ]; then
  echo "FAIL $record cannot be read" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  check_sums() { sha256sum -c SHA256SUMS; }
  hash_stdin() { sha256sum | cut -d' ' -f1; }
  hash_file() { sha256sum "$1" | cut -d' ' -f1; }
else
  check_sums() { shasum -a 256 -c SHA256SUMS; }
  hash_stdin() { shasum -a 256 | cut -d' ' -f1; }
  hash_file() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

status=0
echo "downloaded artifact: $dir"
echo "--- sha256sum -c SHA256SUMS (the bytes as downloaded)"
if ! (cd "$dir" && check_sums); then
  echo "FAIL the downloaded bytes do not match SHA256SUMS" >&2
  status=1
fi

named=$(sed -n 's/^Artifact: .* \([0-9a-f]\{64\}\)$/\1/p' "$record" | head -1)
if [ -z "$named" ]; then
  echo "FAIL $record has no Artifact line ending in a sha256" >&2
  exit 1
fi
echo "--- the record names $named"

tree=$(ls "$dir"/*.tree 2>/dev/null | head -1)
if [ -n "$tree" ]; then
  name=$(basename "$tree" .tree)
  recomputed=$(cd "$dir/$name" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | while read -r f; do
    printf '%s  %s\n' "$(hash_file "$f")" "$f"
  done | hash_stdin)
  echo "tree hash recomputed from $name/: $recomputed"
  [ "$recomputed" = "$named" ] || { echo "FAIL the recomputed tree hash is not the record's" >&2; status=1; }
  [ "$(cat "$tree")" = "$named" ] || { echo "FAIL $(basename "$tree") is not the record's tree hash" >&2; status=1; }
else
  # the one binary named in SHA256SUMS
  file=$(awk 'NR == 1 { print $2 }' "$dir/SHA256SUMS")
  if [ -z "$file" ] || [ ! -f "$dir/$file" ]; then
    echo "FAIL SHA256SUMS names no file that is here" >&2
    exit 1
  fi
  got=$(hash_file "$dir/$file")
  echo "sha256 of $file as downloaded: $got"
  [ "$got" = "$named" ] || { echo "FAIL the downloaded $file is not the sha256 the record names" >&2; status=1; }
fi
[ "$status" -ne 0 ] || echo "ok   the artifact as downloaded is the artifact the record describes"
exit $status
