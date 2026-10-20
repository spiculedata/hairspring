#!/bin/sh
# Unpack the two official toolchains and build a third from source: go1.24.5 plus only the non-test hunks of the
# upstream database/sql fix (patches/fix-only.patch: src/database/sql/sql.go and convert.go).
#
#   .cache/toolchains/go1.24.5          official darwin-arm64 archive, unpacked
#   .cache/toolchains/go1.24.6          official darwin-arm64 archive, unpacked
#   .cache/toolchains/go1.24.5-stock-src  the official 1.24.5 source tree, untouched (the reference for the diff check)
#   .cache/toolchains/go1.24.5-fixonly  that source tree plus the patch, built with make.bash
#
# The bootstrap compiler is the official go1.24.5 toolchain, not whatever `go` is on PATH. The patched tree keeps
# its VERSION file (go1.24.5), so the only difference between its binaries and stock 1.24.5 is database/sql.
# Run scripts/fetch-go.sh first. Idempotent: an existing directory is left alone.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
dl="$here/.cache/dl"
tc="$here/.cache/toolchains"
mkdir -p "$tc"

# Check a cached archive against PINS before anything is unpacked from it. A file that fails (or has no pin) is
# deleted, so the next scripts/fetch-go.sh downloads it again.
verify_archive() {
  f=$1
  if [ ! -f "$dl/$f" ]; then
    echo "missing $dl/$f; run scripts/fetch-go.sh" >&2
    exit 1
  fi
  want=$(awk -v f="$f" '$2 == f { print $1 }' "$here/PINS")
  got=$(shasum -a 256 "$dl/$f" | awk '{ print $1 }')
  if [ -z "$want" ] || [ "$got" != "$want" ]; then
    echo "checksum of $f does not match PINS; removing it, run scripts/fetch-go.sh again" >&2
    rm -f "${dl:?}/$f"
    exit 1
  fi
}

unpack_official() {
  name=$1
  if [ -x "$tc/$name/bin/go" ]; then return 0; fi
  verify_archive "$name.darwin-arm64.tar.gz"
  echo "unpack $name"
  rm -rf "${tc:?}/$name.tmp" "${tc:?}/$name"
  mkdir "$tc/$name.tmp"
  tar -xzf "$dl/$name.darwin-arm64.tar.gz" -C "$tc/$name.tmp"
  mv "$tc/$name.tmp/go" "$tc/$name"
  rmdir "$tc/$name.tmp"
}

unpack_source() {
  dest=$1
  if [ -f "$dest/VERSION" ]; then return 0; fi
  verify_archive go1.24.5.src.tar.gz
  echo "unpack source into $dest"
  rm -rf "${dest:?}.tmp" "${dest:?}"
  mkdir "$dest.tmp"
  tar -xzf "$dl/go1.24.5.src.tar.gz" -C "$dest.tmp"
  mv "$dest.tmp/go" "$dest"
  rmdir "$dest.tmp"
}

unpack_official go1.24.5
unpack_official go1.24.6
unpack_source "$tc/go1.24.5-stock-src"

fixonly="$tc/go1.24.5-fixonly"
if [ ! -x "$fixonly/bin/go" ]; then
  unpack_source "$fixonly"
  echo "apply patches/fix-only.patch"
  # Not `git apply`: this directory sits inside a git work tree (it is ignored there), and git apply then silently
  # skips every file as "outside the repository". patch has no such rule. The grep proves the hunks landed.
  (cd "$fixonly" && patch -p1 --no-backup-if-mismatch <"$here/patches/fix-only.patch")
  grep -q 'func (rs \*Rows) scanLocked' "$fixonly/src/database/sql/sql.go" || {
    echo "patch did not change sql.go" >&2
    exit 1
  }
  echo "build $fixonly (make.bash)"
  (
    cd "$fixonly/src"
    env -i HOME="$HOME" PATH="/usr/bin:/bin:/usr/sbin:/sbin" \
      GOROOT_BOOTSTRAP="$tc/go1.24.5" GOTOOLCHAIN=local GOPROXY=off CGO_ENABLED=0 \
      ./make.bash
  )
fi

for name in go1.24.5 go1.24.6 go1.24.5-fixonly; do
  printf '%s: ' "$name"
  GOROOT="$tc/$name" GOTOOLCHAIN=local "$tc/$name/bin/go" version
done
