#!/bin/sh
# Generates the BUILD-RECORD of the Java probe artifact lang-at.jar: the build and native checks of a JDK-only threaded
# program, in one Markdown file.
#
#   build-record-probe-java.sh [out-file]
#
# Run it on Linux with the pinned JDK on PATH or in JAVA_HOME (the manual CI job installs it). It builds the jar twice (two
# build directories, two time zones) and requires equal sha256 (B1), checks that the entries are sorted and carry the fixed
# timestamp, searches the sources for forbidden reads, checks that every JVM flag profile is valid and prints one clean line,
# runs the program natively and checks only the format of its line and its exit status (B4), and runs strace -ff on one run
# per flag profile to check that the line is one write (B5). The probe has no SDK, no property and no fixture window, so B2,
# B3, B6 and B7 do not apply, and there is no alignment checklist (that list is for the AT programs). Nothing here runs
# Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: JAVA_HOME selects the JDK (default: the java on PATH), JDK_PINNED is the build string the JDK must report
# (default Temurin-21.0.12.1+1), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARTIFACT_DIR
# receives a copy of the pinned jar, its manifest, the flag manifest and SHA256SUMS. The producer fields come from the GitHub
# Actions environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_* variables are the
# interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
runs=${RUNS:-50}
jdk_pinned=${JDK_PINNED:-Temurin-21.0.12.1+1}
out=${1:-$root/build/record-java-probe.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

javadir=$root/java
artifact="lang-at.jar"
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
build_one=$work/build-one
build_two=$work/build-two
jar=$build_one/$artifact
if [ -n "${JAVA_HOME:-}" ]; then
  java=$JAVA_HOME/bin/java
  javac=$JAVA_HOME/bin/javac
else
  java=java
  javac=javac
fi

versions() {
  uname -srm
  got=$("$java" -version 2>&1) || return 1
  echo "$got"
  "$javac" -version 2>&1
  [ -r "${JAVA_HOME:-/nonexistent}/release" ] && grep -E '^(IMPLEMENTOR|JAVA_RUNTIME_VERSION|JAVA_VERSION|OS_ARCH)=' "$JAVA_HOME/release"
  if printf '%s\n' "$got" | sh "$here/check-jdk-version.sh" "$jdk_pinned"; then
    echo "ok   the JDK is the pinned $jdk_pinned"
  else
    echo "FAIL the JDK is not the pinned $jdk_pinned"
    return 1
  fi
  command -v strace >/dev/null 2>&1 && strace -V 2>&1 | head -1
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
  echo "build command: make -C examples/language-fixtures/java jar BUILD=<dir> (javac --release 21 -Xlint:all -Werror -encoding UTF-8 -proc:none; jar --create --date=2026-01-01T00:00:00Z --manifest manifest.mf, the class files listed one by one in sorted order)"
  echo "dependencies: none (JDK classes only; the sources import java.* packages and nothing else)"
  grep -h '^import ' "$javadir"/src/langat/*.java | sort -u
  if grep -h '^import ' "$javadir"/src/langat/*.java | grep -v '^import java\.' | grep -q .; then
    echo "FAIL a source imports something that is not in the JDK"
    return 1
  fi
}

# Two builds in different directories and different time zones must give the same bytes.
build_twice() {
  rm -rf "$build_one" "$build_two"
  TZ=UTC make -C "$javadir" jar BUILD="$build_one" JAVA_HOME="${JAVA_HOME:-}" || return 1
  TZ=Pacific/Auckland make -C "$javadir" jar BUILD="$build_two" JAVA_HOME="${JAVA_HOME:-}" || return 1
  first=$(hash_file "$jar")
  second=$(hash_file "$build_two/$artifact")
  equal_hashes "$artifact" "$first" "$second"
}

# The entries are sorted (META-INF first), and every entry carries the fixed timestamp.
jar_layout() {
  unzip -Z1 "$jar" >"$work/entries.txt" || return 1
  cat "$work/entries.txt"
  [ "$(sed -n 1p "$work/entries.txt")" = "META-INF/" ] && [ "$(sed -n 2p "$work/entries.txt")" = "META-INF/MANIFEST.MF" ] ||
    { echo "FAIL the manifest is not first"; return 1; }
  sed 1,2d "$work/entries.txt" >"$work/classes.txt"
  LC_ALL=C sort -c "$work/classes.txt" || { echo "FAIL the class entries are not sorted"; return 1; }
  echo "ok   the entries are in sorted order, the manifest first"
  unzip -Z "$jar" | grep -v '^Archive:\|^Zip file size\|files, ' >"$work/zipinfo.txt"
  cat "$work/zipinfo.txt"
  if [ "$(awk '{ print $7 " " $8 }' "$work/zipinfo.txt" | sort -u | wc -l | tr -d ' ')" -ne 1 ]; then
    echo "FAIL the entries do not all carry one timestamp"
    return 1
  fi
  echo "ok   every entry carries the same timestamp"
  echo "--- META-INF/MANIFEST.MF"
  unzip -p "$jar" META-INF/MANIFEST.MF
}

# The probe reads no seed, /proc, environment, CPU count, process table or wall clock; its only clock reads are the two
# System.nanoTime calls (vtime_ns).
forbidden_reads() {
  sh "$here/check-sources-java.sh" --allow-nano-time "$javadir/src" || return 1
  echo "ok   the probe reads none of them; its only clock reads are the two System.nanoTime calls (vtime_ns)"
  grep -n 'System\.nanoTime' "$javadir"/src/langat/*.java
}

# Every flag profile of the experiment is a valid set of JVM options, and a run under it prints one clean line.
profiles() {
  JAVA=$java sh "$here/check-java-profiles.sh" --exact "$javadir/profiles.tsv" "$jar"
}

# B4, format only: every run exits 0 with an empty stderr and one stdout line of the six-field form. The distinct counts are
# information (natively the thread order and the random value vary from run to run; `extra`, the map order, is a function
# of the JDK alone). A usage error (any argument) exits 64 with nothing on stdout.
native() {
  i=0
  : >"$work/lines.txt"
  while [ "$i" -lt "$runs" ]; do
    "$java" -cp "$jar" langat.Probe >"$work/out.txt" 2>"$work/err.txt"
    rc=$?
    [ "$rc" -eq 0 ] || { echo "FAIL run $i: exit status $rc, want 0"; return 1; }
    [ ! -s "$work/err.txt" ] || { echo "FAIL run $i: something on stderr:"; cat "$work/err.txt"; return 1; }
    sh "$here/check-probe-line.sh" "$work/out.txt" || { echo "FAIL run $i: stdout is not exactly the six-field line:"; cat "$work/out.txt"; return 1; }
    cat "$work/out.txt" >>"$work/lines.txt"
    i=$((i + 1))
  done
  echo "ok   $runs runs: exit 0, empty stderr, one line in the six-field form"
  tail -1 "$work/lines.txt"
  for name in lock_order arrival rand extra; do
    awk -v name="$name" '{ for (k = 1; k <= NF; k++) if (index($k, name "=") == 1 && !($k in seen)) { seen[$k] = 1; n++ } } END { print "information: " n " distinct " name " of " NR " runs" }' "$work/lines.txt"
  done
  for arg in x --switch=1.0 ""; do
    "$java" -cp "$jar" langat.Probe "$arg" >"$work/out.txt" 2>"$work/err.txt"
    rc=$?
    [ "$rc" -eq 64 ] && [ ! -s "$work/out.txt" ] && [ -s "$work/err.txt" ] || { echo "FAIL the argument '$arg' is not a usage error (exit 64, usage line on stderr, nothing on stdout)"; return 1; }
  done
  echo "ok   any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace. One run per flag profile, with the flags the experiment uses.
events() {
  tab=$(printf '\t')
  while IFS="$tab" read -r arm flags; do
    case "$arm" in '' | '#'*) continue ;; esac
    echo "--- arm $arm: $flags"
    rm -f "$work"/strace.*
    # shellcheck disable=SC2086  # flags is a list of words on purpose
    strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$java" $flags -cp "$jar" langat.Probe >"$work/stdout.txt" 2>"$work/stderr.txt" </dev/null || return 1
    sh "$here/check-probe-writes.sh" "$work"/strace.* || return 1
  done <"$javadir/profiles.tsv"
}

# The probe's own self-test and native tests, as the producer's JDK runs them.
tests() {
  TZ=UTC make -C "$javadir" test BUILD="$work/build-test" JAVA_HOME="${JAVA_HOME:-}"
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-probe-java.sh."
artifact_name=$artifact
artifact_sha="<not built>"
record_no_checklist=1

say "Scope: this record covers lang-at.jar, a JDK-only program (no SDK, no property) used to check scheduling behaviour of Java"
say "threads. It prints one \`PROBE v1\` line of six fields. Per the checks that apply to a probe, B1 and B4 (format only: the"
say "line matches the six-field form and the exit status is 0) are required; B5 is run as well, once per flag profile. B2, B3,"
say "B6 and B7 belong to the AT programs and do not apply."
say ""
say "Flags: the JVM flag profiles are in \`examples/language-fixtures/java/profiles.tsv\` (arm a and arm b), and the jar is run as"
say "\`<jdk>/bin/java <flags> -cp lang-at.jar langat.Probe\`. The flags are not baked into the jar; the manifest of flags is"
say "uploaded next to the jar as \`lang-at-profiles.tsv\`."
say ""
say "Randomness: \`rand\` is the first value of \`new java.util.Random()\`. In JDK 21 that constructor mixes a static counter with"
say "\`System.nanoTime()\`, so \`rand\` follows the monotonic clock, not an operating-system entropy source. \`extra\` hashes the"
say "iteration order of a \`ConcurrentHashMap\` of 32 string keys, which depends on the keys' hash codes and the table only, so it"
say "is the same in every run on one JDK."
say ""
say "Reproducibility: the class files come from \`javac --release 21\`; the jar is written with \`jar --date=2026-01-01T00:00:00Z\`"
say "and its class entries listed one by one in sorted order. B1 builds it twice, in two build directories and two time zones."
say "The JDK is part of the build: another JDK build can give another jar. The pinned artifact comes from one named producer:"
say "the one named above."
say ""
step B1 "toolchain, build command and class count" versions
step B1 "the build, twice (two directories, two time zones), with equal sha256" build_twice
artifact_sha=$(publish_artifact "$jar" "$artifact") || artifact_sha="<not built>"
if [ -f "$jar" ] && [ -n "${ARTIFACT_DIR:-}" ]; then
  publish_artifact "$javadir/manifest.mf" "lang-at.manifest.mf" >/dev/null
  publish_artifact "$javadir/profiles.tsv" "lang-at-profiles.tsv" >/dev/null
fi
step B1 "the jar's layout: sorted entries, one timestamp, the manifest" jar_layout
not_applicable B2 "disassembly of the busy loop and the window" "Not applicable: the probe has no fixture window to prove. Its unlocked counter exists to give the scheduler something to interleave, and its value is not part of the line."
not_applicable B3 "seccomp self-test" "Not applicable: the probe has no no-syscall window."
step F1 "forbidden reads: no seed, /proc, environment, CPU count, process table or wall clock in the probe's sources (two System.nanoTime reads allowed)" forbidden_reads
step F2 "the flag profiles: every flag is valid, no JVM warning, one clean line under each" profiles
step B4 "$runs native runs: the line matches the six-field form and the exit status is 0; any argument is a usage error" native
if command -v strace >/dev/null 2>&1; then
  step B5 "the result line is exactly one write to stdout under each flag profile, and no protocol line exists" events
else
  not_run B5 "the result line is exactly one write" "strace is not installed here."
fi
not_applicable B6 "the SDK's conformance" "Not applicable: the probe does not use the SDK."
not_applicable B7 "wide differs from buggy only in the loop count" "Not applicable: the probe has no modes."
step T1 "the probe's own self-test and native tests" tests

finish_record
exit $failed
