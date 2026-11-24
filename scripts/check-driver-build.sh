#!/usr/bin/env bash
# Checks that the session driver, built the way a session builds it, has no stand-in programs in it.
#
#   scripts/check-driver-build.sh
#
# The driver is built with `--features driver` and nothing else, into a scratch target directory (so it cannot be mixed up
# with the copy the tests build, which has the `fake-vmm` feature on), and asked `--build-info`. The check fails unless the
# answer is `"fake_vmm":false`. Exit 0 when it is, 1 when it is not, 2 when the build or the question fails.
#
# HS_CARGO names another cargo command (the tests use a fake that "builds" a script).
set -u

CARGO="${HS_CARGO:-cargo}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-driver-build.XXXXXX")"
trap 'rm -rf "$TARGET"' EXIT

if ! (cd "$HERE/.." && CARGO_TARGET_DIR="$TARGET" "$CARGO" build -p hairspring-dhyve --features driver --bin dhyve-session-driver); then
  echo "the driver did not build" >&2
  exit 2
fi
DRIVER="$TARGET/debug/dhyve-session-driver"
[ -x "$DRIVER" ] || { echo "no driver at $DRIVER" >&2; exit 2; }
if ! info="$("$DRIVER" --build-info 2>&1)"; then
  echo "the driver would not say what it is built with: $info" >&2
  exit 2
fi
if printf '%s' "$info" | tr -d ' ' | grep -q '"fake_vmm":false'; then
  echo "OK: the driver has no stand-in programs built in"
  exit 0
fi
echo "FAIL: the driver reports $info" >&2
exit 1
