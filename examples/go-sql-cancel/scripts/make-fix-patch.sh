#!/bin/sh
# Rebuild patches/fix-only.patch from upstream and compare it with the committed copy.
#
#   scripts/make-fix-patch.sh          download golang/go commit 83b4a5db2409, keep only the sql.go and convert.go hunks,
#                                      and fail if the result differs from patches/fix-only.patch (after its preamble)
#
# Needs curl, git and the go1.24.5 source archive (scripts/fetch-go.sh). Works in a temporary git repository outside
# this work tree, because `git apply` inside a nested ignored directory silently skips files.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
commit=83b4a5db240960720e51b7d5a6da1f399bd868ee
src="$here/.cache/dl/go1.24.5.src.tar.gz"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/full.patch" "https://github.com/golang/go/commit/$commit.patch"
head -1 "$work/full.patch" | grep -q "^From $commit " || {
  echo "FAIL upstream patch does not start with commit $commit" >&2
  exit 1
}

tar -xzf "$src" -C "$work" go/src/database/sql
(
  cd "$work/go"
  git init -q .
  git add -A >/dev/null
  git -c user.email=none@example.invalid -c user.name=none commit -qm base
  git apply --include=src/database/sql/sql.go --include=src/database/sql/convert.go "$work/full.patch"
  git diff >"$work/fix-only.patch"
)

sed -n '/^diff --git/,$p' "$here/patches/fix-only.patch" >"$work/committed.body"
if cmp -s "$work/committed.body" "$work/fix-only.patch"; then
  echo "ok   patches/fix-only.patch matches the non-test hunks of $commit"
else
  echo "FAIL patches/fix-only.patch differs from the regenerated patch" >&2
  diff "$work/committed.body" "$work/fix-only.patch" >&2 || true
  exit 1
fi
