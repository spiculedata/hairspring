#!/bin/sh
# Stages the Python fixture package and lists it with a hash of every file and a tree hash (build-record check B1).
#
#   stage-python.sh <dest-dir> <listing-file>
#
# The acceptance run pins a package, not a single module: the fixture (at_py.py), the probe (probe_py.py) and the SDK
# package (hairspring/*.py, py.typed), copied side by side into <dest-dir> exactly as they are put on the box. The
# listing has one `<sha256>  <relative path>` line per staged file, sorted by path, and ends with `tree  <sha256>`, the
# sha256 of those lines: one value that identifies the whole package. No bytecode is staged.
# FIXTURE_DIR and SDK_SRC override where the sources come from (the tests use that).
# Exits 0 on success, 1 on a failure, 64 for a usage error.

set -u

if [ $# -ne 2 ]; then
  echo "usage: $0 <dest-dir> <listing-file>" >&2
  exit 64
fi
dest=$1
listing=$2
here=$(cd "$(dirname "$0")" && pwd)
fixture_dir=${FIXTURE_DIR:-$here/../python}
sdk_src=${SDK_SRC:-$here/../../../sdks/python/src}

if command -v sha256sum >/dev/null 2>&1; then
  hash_file() { sha256sum "$1" | cut -d' ' -f1; }
  hash_stdin() { sha256sum | cut -d' ' -f1; }
else
  hash_file() { shasum -a 256 "$1" | cut -d' ' -f1; }
  hash_stdin() { shasum -a 256 | cut -d' ' -f1; }
fi

rm -rf "$dest"
mkdir -p "$dest/hairspring" || exit 1
for f in at_py.py probe_py.py; do
  cp "$fixture_dir/$f" "$dest/$f" || exit 1
done
(cd "$sdk_src/hairspring" && find . -type f \( -name '*.py' -o -name py.typed \) ! -path '*__pycache__*') |
  while read -r f; do
    cp "$sdk_src/hairspring/$f" "$dest/hairspring/$f" || exit 1
  done
[ -f "$dest/hairspring/__init__.py" ] || { echo "the SDK package was not found under $sdk_src" >&2; exit 1; }

body=$(cd "$dest" && find . -type f | sed 's|^\./||' | LC_ALL=C sort |
  while read -r f; do printf '%s  %s\n' "$(hash_file "$dest/$f")" "$f"; done)
printf '%s\n' "$body" >"$listing" || exit 1
tree=$(printf '%s\n' "$body" | hash_stdin)
printf 'tree  %s\n' "$tree" >>"$listing"
echo "staged $(printf '%s\n' "$body" | wc -l | tr -d ' ') files into $dest; tree $tree"
