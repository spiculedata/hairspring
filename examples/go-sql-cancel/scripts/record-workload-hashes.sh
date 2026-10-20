#!/bin/sh
# Build the sqlcancel workload with each toolchain, twice each, in both variants (the default build, whose shim
# writes the full protocol report, and -tags hs_violation_only), as static linux/amd64 binaries, and append their
# sha256 to BUILD-RECORD under a marker line. Fails if a pair of builds differs.
#
#   scripts/record-workload-hashes.sh
#
# Everything above the marker (the toolchain and hello/hellosql entries written by record-hashes.sh) is left
# byte-for-byte alone; a rerun replaces only the section below the marker. Running record-hashes.sh rewrites the
# whole file and drops this section, so run this script after it. The environment and flags are the ones
# record-hashes.sh uses. The Mac sha256 is the identity of record: no claim that a linux-built binary is equal.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
tc="$here/.cache/toolchains"
out="$here/build"
record="$here/BUILD-RECORD"
marker="# --- sqlcancel workload (M1c), written by scripts/record-workload-hashes.sh ---"
toolchains="go1.24.5 go1.24.6 go1.24.5-fixonly"

sha() { shasum -a 256 "$1" | awk '{ print $1 }'; }

# build_one <toolchain> <tags or empty> <dest>
build_one() {
  mkdir -p "$(dirname "$3")"
  (
    cd "$here"
    env -i HOME="$HOME" PATH="/usr/bin:/bin" \
      GOROOT="$tc/$1" GOTOOLCHAIN=local GOPROXY=off GOFLAGS= GOCACHE="$out/gocache-$1" \
      CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 \
      "$tc/$1/bin/go" build -trimpath -buildvcs=false -ldflags=-buildid= ${2:+-tags "$2"} -o "$3" ./cmd/sqlcancel
  )
}

section=$(mktemp)
trap 'rm -f "$section"' EXIT
status=0
{
  echo "$marker"
  echo "# Binaries: cmd/sqlcancel, CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 go build -trimpath -buildvcs=false -ldflags=-buildid="
  echo "# 'violation' is the same source built with -tags hs_violation_only. Mode std/yield is a command-line flag, not a build."
  echo
  for t in $toolchains; do
    echo "[workload $t]"
    for variant in default violation; do
      tags=""
      name="sqlcancel"
      if [ "$variant" = violation ]; then tags="hs_violation_only"; name="sqlcancel_violation"; fi
      build_one "$t" "$tags" "$out/$t/a/$name"
      build_one "$t" "$tags" "$out/$t/b/$name"
      a=$(sha "$out/$t/a/$name")
      b=$(sha "$out/$t/b/$name")
      if [ "$a" = "$b" ]; then
        echo "$a  $name (linux/amd64 static, built twice, identical)"
      else
        echo "MISMATCH  $name: $a vs $b"
        status=1
      fi
    done
    echo
  done
} >"$section"

# Keep everything above an existing marker; otherwise keep the whole file.
base=$(mktemp)
trap 'rm -f "$section" "$base"' EXIT
awk -v m="$marker" '$0 == m { exit } { print }' "$record" >"$base"
# The earlier content ends with one blank line; drop trailing blank lines, then add exactly one.
{ awk 'NF { for (; blanks > 0; blanks--) print ""; print; next } { blanks++ }' "$base"; echo; cat "$section"; } >"$record.tmp"
mv "$record.tmp" "$record"
cat "$section"
exit "$status"
