#!/bin/sh
# Tests for check-patched-diff.sh on small synthetic trees. Needs no Go toolchain.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
check="$here/check-patched-diff.sh"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

failed=0
expect() {
  name=$1
  want=$2
  shift 2
  if "$@" >"$work/out" 2>&1; then got=0; else got=$?; fi
  if [ "$got" = "$want" ]; then
    echo "ok   $name"
  else
    echo "FAIL $name: exit $got, wanted $want"
    sed 's/^/     /' "$work/out"
    failed=1
  fi
}

# new_tree DIR: a minimal stand-in for a Go source tree.
new_tree() {
  mkdir -p "$1/src/database/sql" "$1/src/runtime"
  echo "stock sql" >"$1/src/database/sql/sql.go"
  echo "stock convert" >"$1/src/database/sql/convert.go"
  echo "stock runtime" >"$1/src/runtime/proc.go"
  echo "go1.24.5" >"$1/VERSION"
}

# 1. exactly the two files changed: passes, even with generated files and build outputs added.
new_tree "$work/a-stock"
new_tree "$work/a-patched"
echo "patched sql" >"$work/a-patched/src/database/sql/sql.go"
echo "patched convert" >"$work/a-patched/src/database/sql/convert.go"
mkdir -p "$work/a-patched/src/internal"
echo "generated" >"$work/a-patched/src/internal/zbootstrap.go"
mkdir -p "$work/a-patched/bin" "$work/a-patched/pkg"
echo "binary" >"$work/a-patched/bin/go"
expect "exactly the two files" 0 "$check" "$work/a-stock" "$work/a-patched"

# 2. identical trees: neither file differs, so the patch did not apply.
new_tree "$work/b-stock"
new_tree "$work/b-patched"
expect "identical trees fail" 1 "$check" "$work/b-stock" "$work/b-patched"

# 3. only one of the two files changed.
new_tree "$work/c-stock"
new_tree "$work/c-patched"
echo "patched sql" >"$work/c-patched/src/database/sql/sql.go"
expect "only sql.go changed fails" 1 "$check" "$work/c-stock" "$work/c-patched"

# 4. a third file changed as well.
new_tree "$work/d-stock"
new_tree "$work/d-patched"
echo "patched sql" >"$work/d-patched/src/database/sql/sql.go"
echo "patched convert" >"$work/d-patched/src/database/sql/convert.go"
echo "patched runtime" >"$work/d-patched/src/runtime/proc.go"
expect "a third changed file fails" 1 "$check" "$work/d-stock" "$work/d-patched"

# 5. a file deleted from the patched tree.
new_tree "$work/e-stock"
new_tree "$work/e-patched"
echo "patched sql" >"$work/e-patched/src/database/sql/sql.go"
echo "patched convert" >"$work/e-patched/src/database/sql/convert.go"
rm "$work/e-patched/src/runtime/proc.go"
expect "a deleted file fails" 1 "$check" "$work/e-stock" "$work/e-patched"

# 6. an extra source file that is not a generated one.
new_tree "$work/f-stock"
new_tree "$work/f-patched"
echo "patched sql" >"$work/f-patched/src/database/sql/sql.go"
echo "patched convert" >"$work/f-patched/src/database/sql/convert.go"
echo "extra" >"$work/f-patched/src/runtime/extra.go"
expect "an extra non-generated file fails" 1 "$check" "$work/f-stock" "$work/f-patched"

exit "$failed"
