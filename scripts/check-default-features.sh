#!/usr/bin/env bash
# Checks that a default workspace build leaves the experimental dhyve feature off in the shared crates.
#
#   scripts/check-default-features.sh
#
# Cargo unifies features across one `--workspace` invocation, so a feature that one member turns on in a shared crate is on
# for every member of that build. This runs `cargo tree --workspace -e features -i CRATE` for each shared crate and fails
# when the output holds the feature node `CRATE feature "dhyve"`. It matches that node and nothing else: the inverted tree
# also lists every workspace package that depends on CRATE, and the experimental crate's own name contains the word.
# Exit 0 when no node is found, 1 when one is, 2 when cargo itself fails.
#
# HS_CARGO names another cargo command (the tests use a fake).
set -u

CARGO="${HS_CARGO:-cargo}"
FEATURE="dhyve"
CRATES="hairspring-core hairspring-engine hairspring-service hairspring-cli"

status=0
for crate in $CRATES; do
  if ! tree="$("$CARGO" tree --workspace -e features -i "$crate" 2>&1)"; then
    echo "cargo tree failed for $crate:" >&2
    printf '%s\n' "$tree" | sed 's/^/    | /' >&2
    exit 2
  fi
  # The crate name must start the node (after the tree's drawing characters), and the feature name must end it.
  if printf '%s\n' "$tree" | grep -Eq "(^|[^A-Za-z0-9_-])${crate} feature \"${FEATURE}\"( \(\*\))?\$"; then
    echo "FAIL: the feature \"$FEATURE\" is on for $crate in a default workspace build" >&2
    status=1
  fi
done
[ "$status" -ne 0 ] || echo "OK: the feature \"$FEATURE\" is off for $CRATES in a default workspace build"
exit "$status"
