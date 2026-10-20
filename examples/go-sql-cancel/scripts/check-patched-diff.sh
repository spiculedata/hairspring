#!/bin/sh
# Check that the patched Go tree differs from the stock go1.24.5 tree in exactly two files:
# src/database/sql/sql.go and src/database/sql/convert.go.
#
#   scripts/check-patched-diff.sh [STOCK_DIR PATCHED_DIR]
#
# Defaults are the trees build-patched-toolchain.sh leaves under .cache/toolchains. Exit 0 only when the set of files
# that differ is exactly the expected pair, no file of the stock tree is missing from the patched one, and no extra
# file appears outside the build outputs (bin, pkg, generated z* files and the like). Everything else exits 1.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
tc="$here/.cache/toolchains"
stock=${1:-$tc/go1.24.5-stock-src}
patched=${2:-$tc/go1.24.5-fixonly}

expected="src/database/sql/convert.go
src/database/sql/sql.go"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Paths relative to a tree root, without the build outputs make.bash writes.
list_files() {
  (cd "$1" && find . -type f ! -path './bin/*' ! -path './pkg/*' | sed 's|^\./||' | sort)
}

list_files "$stock" >"$work/stock.list"
list_files "$patched" >"$work/patched.list"

# Present in the stock tree, absent from the patched one: always a failure.
comm -23 "$work/stock.list" "$work/patched.list" >"$work/missing"
# Only in the patched tree: generated files from make.bash are expected (z-prefixed or under cmd/dist); report them.
comm -13 "$work/stock.list" "$work/patched.list" >"$work/added"

: >"$work/differ"
while IFS= read -r f; do
  if ! cmp -s "$stock/$f" "$patched/$f"; then echo "$f" >>"$work/differ"; fi
done <"$work/stock.list"
# A file missing from the patched tree cannot be compared; it is reported through $work/missing.
if [ -s "$work/missing" ]; then
  while IFS= read -r f; do grep -vxF "$f" "$work/differ" >"$work/differ.next" || true; mv "$work/differ.next" "$work/differ"; done <"$work/missing"
fi
sort "$work/differ" -o "$work/differ"

status=0
if [ -s "$work/missing" ]; then
  echo "FAIL files missing from the patched tree:"
  sed 's/^/  /' "$work/missing"
  status=1
fi

unexpected_added=$(grep -vE '(^|/)z[A-Za-z0-9_]*\.go$|^src/cmd/dist/dist$|^src/cmd/cgo/zdefaultcc\.go$' "$work/added" || true)
if [ -n "$unexpected_added" ]; then
  echo "FAIL files only in the patched tree that are not generated build files:"
  printf '%s\n' "$unexpected_added" | sed 's/^/  /'
  status=1
fi

if [ "$(cat "$work/differ")" = "$expected" ]; then
  echo "ok   differing files are exactly:"
  sed 's/^/  /' "$work/differ"
else
  echo "FAIL differing files are not exactly the expected pair. expected:"
  printf '%s\n' "$expected" | sed 's/^/  /'
  echo "  got:"
  sed 's/^/  /' "$work/differ"
  status=1
fi

exit "$status"
