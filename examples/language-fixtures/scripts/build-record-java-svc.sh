#!/bin/sh
# A short build record of the Java service fixture artifact svc.jar (class langsvc.Main, the Java SDK shaded in).
#
#   build-record-java-svc.sh <out-file>
#
# Run it on Linux with the pinned JDK on PATH or in JAVA_HOME. It checks that the JDK is the pinned build, builds the jar
# twice (two build directories, two time zones) and requires equal sha256, checks that the entries are sorted and carry
# the fixed timestamp, runs the source checks and the window check, and copies the jar, its manifest and SHA256SUMS to
# ARTIFACT_DIR when that is set. It does not run the native self-checks (the native targets do) and never runs Hermit.
#
# Environment: JAVA_HOME selects the JDK (default: the java on PATH), JDK_PINNED is the build string the JDK must report
# (default Temurin-21.0.12.1+1), ARTIFACT_DIR receives the artifact. Exits 0 when every check passed, 1 otherwise, 64 for
# a usage error.

set -u

if [ $# -ne 1 ]; then
  echo "usage: $0 <out-file>" >&2
  exit 64
fi
out=$1
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
javadir=$root/java
jdk_pinned=${JDK_PINNED:-Temurin-21.0.12.1+1}
java=${JAVA_HOME:+$JAVA_HOME/bin/}java
jar=${JAVA_HOME:+$JAVA_HOME/bin/}jar
work=$(mktemp -d "${TMPDIR:-/tmp}/svc-record.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
mkdir -p "$(dirname "$out")" || exit 1
failed=0
: >"$out"
say() { printf '%s\n' "$*" >>"$out"; }
check() { # check <name> <command...>: runs the command, appends its output and the result to the record
  name=$1
  shift
  say "### $name"
  say '```'
  if "$@" >>"$out" 2>&1; then
    say '```'
    say "PASS $name"
  else
    say '```'
    say "FAIL $name"
    failed=1
  fi
  say ""
}

jdk() { "$java" -version 2>&1 && "$java" -version 2>&1 | sh "$here/check-jdk-version.sh" "$jdk_pinned"; }

twice() {
  TZ=UTC make -C "$javadir" svc-jar SVC_BUILD="$work/one" JAVA_HOME="${JAVA_HOME:-}" >/dev/null || return 1
  TZ=Pacific/Auckland make -C "$javadir" svc-jar SVC_BUILD="$work/two" JAVA_HOME="${JAVA_HOME:-}" >/dev/null || return 1
  one=$(sha256sum "$work/one/svc.jar" | cut -d' ' -f1)
  two=$(sha256sum "$work/two/svc.jar" | cut -d' ' -f1)
  echo "build one (TZ=UTC):              $one"
  echo "build two (TZ=Pacific/Auckland): $two"
  [ "$one" = "$two" ]
}

layout() {
  "$jar" --list --verbose --file "$work/one/svc.jar" >"$work/list" || return 1
  awk '{print $NF}' "$work/list" | grep '\.class$' >"$work/names"
  LC_ALL=C sort "$work/names" | cmp -s - "$work/names" || { echo "class entries are not in sorted order"; return 1; }
  dates=$(awk '{print $2, $3, $4, $5, $6, $7}' "$work/list" | sort -u | wc -l | tr -d ' ')
  echo "entries: $(wc -l <"$work/list" | tr -d ' '), class entries: $(wc -l <"$work/names" | tr -d ' '), distinct timestamps: $dates"
  [ "$dates" -eq 1 ] || { echo "more than one entry timestamp"; return 1; }
  awk '{print $NF}' "$work/list" | grep -qx 'META-INF/MANIFEST.MF' || { echo "no manifest"; return 1; }
  unzip -p "$work/one/svc.jar" META-INF/MANIFEST.MF | grep -q '^Main-Class: langsvc.Main' || { echo "the manifest does not name langsvc.Main"; return 1; }
  grep -qx 'langsvc/Main.class' "$work/names" && grep -qx 'hairspring/Hairspring.class' "$work/names"
}

sources() { make -C "$javadir" svc-sources svc-window; }

say "# Build record: Java service fixture (svc.jar, class langsvc.Main)"
say ""
say "- source commit: ${SOURCE_COMMIT:-$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)}"
say "- producer: ${PRODUCER:-unknown}"
say "- pinned JDK: $jdk_pinned"
say ""
check "the JDK is the pinned build" jdk
check "the jar, built twice (two directories, two time zones), with equal sha256" twice
check "the jar's layout: sorted class entries, one timestamp, the manifest, the shaded SDK" layout
check "source checks and the window check" sources

if [ "$failed" -eq 0 ] && [ -n "${ARTIFACT_DIR:-}" ]; then
  mkdir -p "$ARTIFACT_DIR" || exit 1
  cp "$work/one/svc.jar" "$ARTIFACT_DIR/svc.jar"
  cp "$javadir/svc/manifest.mf" "$ARTIFACT_DIR/manifest.mf"
  (cd "$ARTIFACT_DIR" && sha256sum svc.jar manifest.mf >SHA256SUMS)
  say "artifact: $(cd "$ARTIFACT_DIR" && cat SHA256SUMS | tr '\n' ' ')"
fi
[ "$failed" -eq 0 ] && say "RESULT PASS" || say "RESULT FAIL"
exit "$failed"
