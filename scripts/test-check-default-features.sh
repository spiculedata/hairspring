#!/usr/bin/env bash
# Tests for scripts/check-default-features.sh, with a fake cargo that prints a fixed `cargo tree` output.
#
#   bash scripts/test-check-default-features.sh
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/check-default-features.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-feature-guard-tests.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
FAILED=0

# The fake prints the file FAKE_TREE (or fails, when FAKE_FAIL is set) whatever it is asked.
cat >"$TMP/cargo" <<'FAKE'
#!/usr/bin/env bash
[ -z "${FAKE_FAIL:-}" ] || { echo "error: could not read the workspace" >&2; exit 101; }
cat "$FAKE_TREE"
FAKE
chmod +x "$TMP/cargo"

expect() { # NAME EXPECTED_RC ACTUAL_RC [OUTPUT_PATTERN]
  if [ "$2" != "$3" ]; then
    echo "FAIL $1: exit $3, expected $2"
    sed 's/^/    | /' "$TMP/out"
    FAILED=1
  elif [ -n "${4:-}" ] && ! grep -q -- "$4" "$TMP/out"; then
    echo "FAIL $1: output lacks '$4'"
    sed 's/^/    | /' "$TMP/out"
    FAILED=1
  else
    echo "ok   $1"
  fi
}

run() { FAKE_TREE="$1" HS_CARGO="$TMP/cargo" bash "$SCRIPT" >"$TMP/out" 2>&1; }

# What a default build prints: the crate that carries the word as a dependent, and no feature node for it.
cat >"$TMP/default" <<'TREE'
hairspring-core v0.1.0 (/repo/crates/hairspring-core)
└── hairspring-core feature "default" (command-line)
    ├── hairspring-cli v0.1.0 (/repo/crates/hairspring-cli)
    │   └── hairspring-cli feature "default" (command-line)
    ├── hairspring-dhyve v0.1.0 (/repo/crates/hairspring-dhyve)
    │   └── hairspring-dhyve feature "default" (command-line)
    └── hairspring-engine v0.1.0 (/repo/crates/hairspring-engine)
        └── hairspring-engine feature "default" (command-line)
TREE
run "$TMP/default"; expect "a default tree passes, though a crate name holds the word" 0 $? "OK:"

# A shared crate with the feature on.
printf '%s\n' 'hairspring-core v0.1.0 (/repo/crates/hairspring-core)' \
  '├── hairspring-core feature "default" (command-line)' \
  '└── hairspring-core feature "dhyve"' >"$TMP/on"
run "$TMP/on"; expect "the feature node on a shared crate fails" 1 $? 'the feature "dhyve" is on for hairspring-core'

printf '%s\n' '    │   └── hairspring-engine feature "dhyve" (*)' >"$TMP/seen"
run "$TMP/seen"; expect "a repeated node (marked with a star) fails" 1 $? 'hairspring-engine'

printf '%s\n' 'hairspring-cli feature "dhyve"' >"$TMP/cli"
run "$TMP/cli"; expect "the feature on the command line crate fails" 1 $? 'hairspring-cli'

# Lookalikes that are not the node.
printf '%s\n' 'hairspring-core feature "dhyve-extra"' \
  'other-hairspring-core feature "dhyve"' \
  'hairspring-core feature "dhyved"' \
  'hairspring-core dhyve' >"$TMP/lookalike"
run "$TMP/lookalike"; expect "feature names and crate names that only look like it pass" 0 $? "OK:"

FAKE_FAIL=1 FAKE_TREE="$TMP/default" HS_CARGO="$TMP/cargo" bash "$SCRIPT" >"$TMP/out" 2>&1
expect "a cargo that fails is a failure of its own" 2 $? "could not read the workspace"

[ "$FAILED" -eq 0 ] && echo "all passed"
exit "$FAILED"
