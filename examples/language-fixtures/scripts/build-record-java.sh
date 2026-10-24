#!/bin/sh
# Generates the BUILD-RECORD of the Java fixture artifact lang-at.jar (class langat.Main, the Java SDK shaded in): checks B1
# to B7 of the fixture contract, in one Markdown file.
#
#   build-record-java.sh [out-file]
#
# Run it on Linux with the pinned JDK on PATH or in JAVA_HOME (the manual CI job installs it). It builds the jar twice (two
# build directories, two time zones) and requires equal sha256 (B1), checks that the entries are sorted and carry the fixed
# timestamp, reads the window off the bytecode (B2: what the source compiles to; the JIT is not covered, see the record),
# searches the sources for forbidden reads, checks that every JVM flag profile is valid and runs the program under it,
# runs the native self-check 50 times per mode (B4), runs strace -ff on one buggy run per profile and without flags (B5),
# runs the SDK's own tests and conformance (B6) and prints the source of the modes (B7). It then captures the evidence for
# the rest of the alignment checklist from the built artifact and ticks each checklist line only when the evidence that
# proves it passed (record-lib.sh). Nothing here runs Hermit or touches the box. Exits 1 if any step failed.
#
# Environment: JAVA_HOME selects the JDK (default: the java on PATH), JDK_PINNED is the build string the JDK must report
# (default Temurin-21.0.12.1+1), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARM_RUNS the
# buggy runs per flag profile (default 5), ARTIFACT_DIR receives a copy of the pinned jar, its manifest, the flag manifest
# and SHA256SUMS. The producer fields come from the GitHub Actions environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_*, evidence_*
# and barrier_sources variables are the interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
runs=${RUNS:-50}
arm_runs=${ARM_RUNS:-5}
jdk_pinned=${JDK_PINNED:-Temurin-21.0.12.1+1}
out=${1:-$root/build/record-java.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

javadir=$root/java
sdkdir=$repo/sdks/java
artifact="lang-at.jar"
lang=java
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
build_one=$work/build-one
build_two=$work/build-two
jar=$build_one/$artifact
if [ -n "${JAVA_HOME:-}" ]; then
  java=$JAVA_HOME/bin/java
  javac=$JAVA_HOME/bin/javac
  javap=$JAVA_HOME/bin/javap
else
  java=java
  javac=javac
  javap=javap
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
  echo "build command: make -C examples/language-fixtures/java at-jar AT_BUILD=<dir> (javac --release 21 -Xlint:all -Werror -encoding UTF-8 -proc:none over the SDK's sources, the probe's and the fixture's; jar --create --date=2026-01-01T00:00:00Z --manifest at/manifest.mf, the class files listed one by one in sorted order)"
  echo "dependencies: none (JDK classes only; the sources import java.* packages and the two packages of this repository, hairspring and langat)"
  grep -h '^import ' "$sdkdir"/src/hairspring/*.java "$javadir"/src/langat/*.java "$javadir"/at/src/langat/*.java | sort -u
  if grep -h '^import ' "$sdkdir"/src/hairspring/*.java "$javadir"/src/langat/*.java "$javadir"/at/src/langat/*.java | grep -v '^import java\.' | grep -v '^import hairspring\.' | grep -q .; then
    echo "FAIL a source imports something that is not in the JDK"
    return 1
  fi
}

# Two builds in different directories and different time zones must give the same bytes.
build_twice() {
  rm -rf "$build_one" "$build_two"
  TZ=UTC make -C "$javadir" at-jar AT_BUILD="$build_one" JAVA_HOME="${JAVA_HOME:-}" || return 1
  TZ=Pacific/Auckland make -C "$javadir" at-jar AT_BUILD="$build_two" JAVA_HOME="${JAVA_HOME:-}" || return 1
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
  grep -q '^langat/Main.class$' "$work/classes.txt" && grep -q '^langat/Probe.class$' "$work/classes.txt" && grep -q '^hairspring/Hairspring.class$' "$work/classes.txt" ||
    { echo "FAIL the jar does not hold langat.Main, langat.Probe and the shaded SDK"; return 1; }
  if grep -q 'module-info' "$work/classes.txt"; then
    echo "FAIL the jar holds a module descriptor"
    return 1
  fi
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

# B2 on the bytecode: what the source compiles to. The JIT is covered by the note below the step, not by this check.
window() {
  "$javap" -c -p -cp "$build_one/classes" langat.Race >"$work/javap.txt" || return 1
  sh "$here/check-window-java.sh" "$work/javap.txt"
}

forbidden_reads() {
  sh "$here/check-sources-java.sh" "$javadir/at/src" || return 1
  echo "ok   the fixture reads no seed, /proc, environment, CPU count, process table or clock (the SDK, a library, reads its own HAIRSPRING_* variables and its own process's auxiliary vector)"
}

# Every flag profile of the experiment is a valid set of JVM options, and the program runs under it: fixed is exact, the
# result lines are clean, and how often buggy fired is printed.
profiles() {
  JAVA=$java ARM_RUNS=$arm_runs sh "$here/check-java-arms.sh" --exact "$javadir/profiles.tsv" "$jar"
}

# B4: the whole contract natively, 50 runs of buggy and fixed, wide as a contract check, a negative control. The default
# JVM flags (the JIT on), as a person would run it; the two flag profiles are exercised by the previous step.
native() {
  make -C "$javadir" "$build_one/at-java" AT_BUILD="$build_one" JAVA_HOME="${JAVA_HOME:-}" >/dev/null || return 1
  LANG_AT_REQUIRE_PARALLEL=1 sh "$here/run-checks.sh" "$lang" "$build_one/at-java" "$runs"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace. One run without flags and one per flag profile. The SDK writes each protocol line to fd 2
# with one call of FileOutputStream.write, the result line goes to fd 1 with one.
events() {
  tab=$(printf '\t')
  {
    printf 'default\t\n'
    grep -v '^#' "$javadir/profiles.tsv" | grep -v '^$'
  } >"$work/event-arms.tsv"
  while IFS="$tab" read -r arm flags; do
    echo "--- arm $arm: ${flags:-(no flags)}"
    rm -f "$work"/strace.*
    # shellcheck disable=SC2086  # flags is a list of words on purpose
    strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$java" $flags -cp "$jar" langat.Main buggy >"$work/stdout.txt" 2>"$work/stderr.txt" </dev/null || return 1
    cat "$work"/strace.* >"$work/strace-all.txt"
    lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
    sh "$here/check-writes.sh" "$work/strace-all.txt" >"$work/writes.txt" || { cat "$work/writes.txt"; return 1; }
    cat "$work/writes.txt"
    written=$(cut -d' ' -f1 "$work/writes.txt")
    echo "HAIRSPRING_EVENT lines on stderr: $lines; protocol line writes in the trace: $written"
    [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ] || return 1
  done <"$work/event-arms.tsv"
}

# The SDK's own tests (the self-test and the sample programs as real processes) and the conformance tests against the
# reference parser, both with the JDK this record names.
goldens() {
  TZ=UTC make -C "$sdkdir" test BUILD="$work/sdk-build" JAVA_HOME="${JAVA_HOME:-}" || return 1
  HS_REQUIRE_JAVA=1 HS_JAVA=$java cargo test --manifest-path "$repo/Cargo.toml" -p hairspring-props --test java_sdk
}

# The unit tests of the fixture's own classes.
fixture_tests() {
  TZ=UTC make -C "$javadir" at-selftest AT_BUILD="$work/build-test" JAVA_HOME="${JAVA_HOME:-}"
}

modes() {
  {
    sed -n '/^enum Mode {/,/^    private final String wire;/p' "$javadir/at/src/langat/Mode.java"
    sed -n '/private void racyIterations/,/^    }/p;/private void lockedIterations/,/^    }/p' "$javadir/at/src/langat/Race.java"
  } | require_lines
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-java.sh."
artifact_name=$artifact
artifact_sha="<not built>"
evidence_artifact=$build_one/at-java
barrier_sources=$javadir/at/src/langat/Race.java
constant_sources=$javadir/at/src/langat/Contract.java

say "Scope: this record covers lang-at.jar built from this checkout: class \`langat.Main\` (the fixture, modes buggy, fixed, wide),"
say "the Java SDK's classes (\`hairspring.*\`), shaded in, and the probe's classes (\`langat.Probe\` and its helpers), which the jar"
say "carries as well and which use no SDK. The jar holds no module descriptor and names \`langat.Main\` as its Main-Class. The"
say "probe-only jar, without the SDK, is a different artifact with its own record (build-record-probe-java.sh)."
say ""
say "Flags: the JVM flag profiles are in \`examples/language-fixtures/java/profiles.tsv\` (arm a and arm b), and the jar is run as"
say "\`<jdk>/bin/java <flags> -cp lang-at.jar langat.Main <mode>\`. The flags are not baked into the jar; the manifest of flags is"
say "uploaded next to the jar as \`lang-at-profiles.tsv\`."
say ""
say "Randomness: \`rand\` is the first value of \`new java.util.Random()\`. In JDK 21 that constructor mixes a static counter with"
say "\`System.nanoTime()\`, so \`rand\` follows the monotonic clock, not an operating-system entropy source; under the engine it may"
say "differ across seeds, and same-seed equality checks clock determinism, not OS entropy."
say ""
say "Reproducibility: the class files come from \`javac --release 21\`; the jar is written with \`jar --date=2026-01-01T00:00:00Z\`"
say "and its class entries listed one by one in sorted order. B1 builds it twice, in two build directories and two time zones."
say "The JDK is part of the build: another JDK build can give another jar. The pinned artifact comes from one named producer:"
say "the one named above."
say ""
say "The JIT and the window (read this before B2): the window is a load of a plain field, a busy loop and a store of that field."
say "Nothing in the language stops a compiler from moving the plain load past the busy loop or keeping the counter in a register,"
say "which would remove the window. The interpreter (arm a) and the C1 compiler (arm b) do not do that; C2 might, for a loop it"
say "compiles, and the worker loops (1,000 iterations) are too short to be compiled by default. B2 below reads the bytecode, which"
say "says what the source compiles to; it cannot say what a JIT makes of it. The native runs (B4) and the profile runs are the"
say "evidence that the race fires for the arm that ran."
say ""
step B1 "toolchain, build command and dependencies" versions
step B1 "the build, twice (two directories, two time zones), with equal sha256" build_twice
artifact_sha=$(publish_artifact "$jar" "$artifact") || artifact_sha="<not built>"
if [ -f "$jar" ] && [ -n "${ARTIFACT_DIR:-}" ]; then
  publish_artifact "$javadir/at/manifest.mf" "lang-at.manifest.mf" >/dev/null
  publish_artifact "$javadir/profiles.tsv" "lang-at-profiles.tsv" >/dev/null
fi
step B1 "the jar's layout: sorted entries, one timestamp, the manifest, the shaded SDK" jar_layout
not_applicable B2 "disassembly of the busy loop and the window" "Not applicable as stated: there is no machine code in the artifact, the JVM compiles it at run time (or interprets it, which is arm a), and what it makes of the window is not recorded here. The next step reads the bytecode instead." yes
step B2 "the window in the bytecode: a load of the counter, one call of spin, a store; no other call, no monitor in the unlocked loop, no allocation" window
not_applicable B3 "seccomp self-test" "Not applicable to Java: the JVM makes system calls on other threads (garbage collector, compiler, signal dispatcher), so a filter that allows only exit would kill it. The no-syscall window is a code-review statement: the window is the bytecode shown in B2, a getfield, one invokestatic of spin and a putfield, none of which calls the operating system in the interpreter or in C1-compiled code." yes
step F1 "forbidden reads: no seed, /proc, environment, CPU count, process table or clock in the fixture's sources" forbidden_reads
step F2 "the flag profiles: every flag is valid, no JVM warning, fixed is exact and the lines are clean under each" profiles
say "Exit-3 paths: a stdout that cannot be written is exercised by B4 (exit 3 required, on /dev/full). A thread that cannot be started"
say "cannot be injected without hook code in the artifact; the fixture's own tests (T1) cover the failure paths of \`Main.run\` with a"
say "workload that throws, and the abort path of the barrier."
say ""
step B4 "$runs native runs of buggy and fixed, wide as a contract check, a negative control (at least two CPUs required)" native
if command -v strace >/dev/null 2>&1; then
  step B5 "each HAIRSPRING_EVENT line is one write(2), and the result line is exactly one write, without flags and under each profile" events
else
  not_run B5 "each HAIRSPRING_EVENT line is one write(2)" "strace is not installed here."
fi
if ! command -v cargo >/dev/null 2>&1; then
  not_run B6 "the SDK's conformance" "cargo is not installed here. Run \`HS_REQUIRE_JAVA=1 cargo test -p hairspring-props --test java_sdk\` on a host with cargo and paste its output."
else
  step B6 "the SDK's own tests and conformance against the reference parser" goldens
fi
step B7 "the modes: wide differs from buggy only in the busy loop length" modes
step T1 "the fixture's own tests" fixture_tests

say "## Evidence for the other checklist lines (captured from the built artifact above, run as \`java -cp lang-at.jar langat.Main\`)"
say ""
evidence_steps

alignment_checklist \
  "One artifact, named \`$artifact\` (class \`langat.Main\`); no separate racy and locked builds, no flag choosing the mode." \
  "argv[1] is buggy, fixed or wide; anything else, or nothing, prints a usage line on stderr and exits 64." \
  "No read of the seed, /proc, the CPU count, the environment or the clock in the fixture's sources outside the runtime and the random source (the SDK, a library, reads its own HAIRSPRING_* variables and its own auxiliary vector)."

finish_record
exit $failed
