#!/bin/sh
# Generates the BUILD-RECORD of the .NET probe artifact probe-dotnet (LangAtProbe.dll with its runtimeconfig.json and
# deps.json): the build and native checks of a BCL-only threaded program, in one Markdown file.
#
#   build-record-probe-dotnet.sh [out-file]
#
# Run it on Linux with the pinned .NET SDK (the manual CI job and build-record-docker.sh do). It publishes the probe twice
# (two copies of the sources in different directories, with different file times) and requires equal sha256 for the dll, the
# runtimeconfig.json and the deps.json (B1), searches the sources for forbidden reads, runs the program natively under the
# base environment and under each environment profile and checks only the format of its line and its exit status (B4), and
# runs strace -ff on one run per profile to check that the line is one write (B5). The probe has no SDK, no property and
# no fixture window, so B2, B3, B6 and B7 do not apply, and there is no alignment checklist (that list is for the AT
# programs). Nothing here runs Hermit or touches the box. Exits 1 if any step failed.
#
# The SDK (DOTNET) only builds. The program is run (B4, B5) on the pinned runtime: by default the runtime tarball
# dotnet-runtime-<RUNTIME_PINNED>-linux-x64.tar.gz, fetched from RUNTIME_URL (or taken from RUNTIME_TARBALL), checked against
# RUNTIME_SHA512 and extracted under a temporary directory, which is DOTNET_ROOT for every run, so the record cannot silently
# run on a newer 10.0.x patch that the machine has installed. RUNTIME_SOURCE=installed runs on the installed runtime instead
# (for a machine that is not x86-64 Linux); the record says which was used, and still requires the version to be pinned.
#
# Environment: DOTNET is the dotnet executable (default dotnet), SDK_PINNED and RUNTIME_PINNED the pinned versions (defaults
# the version in the repository's global.json, 10.0.402, and 10.0.13), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARTIFACT_DIR receives a
# copy of the pinned files and a SHA256SUMS file. The producer fields come from the GitHub Actions environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_* variables are the
# interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
dnroot=$root/dotnet
runs=${RUNS:-50}
dotnet=${DOTNET:-dotnet}
sdk_pinned=${SDK_PINNED:-$(sh "$here/sdk-pin.sh")} || exit 1
runtime_pinned=${RUNTIME_PINNED:-10.0.13}
out=${1:-$root/build/record-probe-dotnet.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

artifact="probe-dotnet"
files="LangAtProbe.dll LangAtProbe.runtimeconfig.json LangAtProbe.deps.json"
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
one=$work/build-one-for-the-first-publish
two=$work/second/build-two-for-the-second-publish
pub_one=$work/publish-one
pub_two=$work/publish-two
dll=$pub_one/LangAtProbe.dll
dotnet_path=$(command -v "$dotnet" 2>/dev/null || echo "$dotnet")
runtime_source=${RUNTIME_SOURCE:-pinned}
# The tarball of the pinned runtime and its sha512 (published by Microsoft; the tarball's sha256 is
# 958c952691d2c3f8ce21f717f4e911c02b5da0e21ead21bd576368c454b79c77).
runtime_url=${RUNTIME_URL:-https://builds.dotnet.microsoft.com/dotnet/Runtime/$runtime_pinned/dotnet-runtime-$runtime_pinned-linux-x64.tar.gz}
runtime_sha512=${RUNTIME_SHA512:-9574d95364110409416735e7842796f8d614c21d74cb25d204a3e4cfd11b2c05efdf19ca89b8069650a04f64add2705967dcb04b8665a36d7a3f30a5d8c05afe}
# The dotnet executable the program runs on, and its runtime directory (DOTNET_ROOT): set by the runtime step (R1); until
# then the installed one.
rt_dotnet=$dotnet_path
droot=$(dirname "$(readlink -f "$dotnet_path" 2>/dev/null || echo "$dotnet_path")")
# The six-field line and nothing else.
line_rx='^PROBE v1 lang=dotnet lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16} vtime_ns=[0-9]{20} extra=[0-9a-f]{16}$'
publish_flags="-c Release -p:Deterministic=true -p:ContinuousIntegrationBuild=true --no-restore --nologo -v q"

if command -v sha256sum >/dev/null 2>&1; then
  hash_stdin() { sha256sum | cut -d' ' -f1; }
else
  hash_stdin() { shasum -a 256 | cut -d' ' -f1; }
fi

# tree_hash <dir>: the sha256 of the sorted `<sha256>  <path>` lines of the files below dir, paths relative to it.
tree_hash() {
  (cd "$1" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | while read -r f; do
    printf '%s  %s\n' "$(hash_file "$f")" "$f"
  done | hash_stdin)
}

versions() {
  uname -srm
  got=$("$dotnet" --version) || return 1
  echo "dotnet SDK $got"
  "$dotnet" --list-runtimes
  echo "image digest: ${IMAGE_DIGEST:-<TO BE FILLED: not given>}"
  command -v strace >/dev/null 2>&1 && strace -V 2>&1 | head -1
  [ "$got" = "$sdk_pinned" ] || { echo "FAIL the SDK is not the pinned $sdk_pinned"; return 1; }
  echo "ok   the SDK is the pinned $sdk_pinned"
  echo "restore command: dotnet restore LangAtProbe.csproj --nologo -v q"
  echo "build command: dotnet publish LangAtProbe.csproj $publish_flags -o <dir>"
  echo "--- the project file (framework-dependent, no packages, no apphost, no debug symbols)"
  cat "$dnroot/src/LangAtProbe.csproj"
  [ "$(grep -c PackageReference "$dnroot/src/LangAtProbe.csproj")" -eq 0 ] || { echo "FAIL the project references packages"; return 1; }
  echo "ok   the project has no PackageReference (BCL only)"
}

# The runtime the program runs on (B4, B5). Pinned: the tarball is fetched (unless RUNTIME_TARBALL names it), checked against
# its sha512 and extracted; the extracted directory is DOTNET_ROOT. Either way the runtime that will actually run the
# program, asked with --list-runtimes, must be exactly the pinned version and the only Microsoft.NETCore.App there.
pinned_runtime() {
  echo "runtime source: $runtime_source"
  case "$runtime_source" in
    pinned)
      [ "$(uname -s)-$(uname -m)" = "Linux-x86_64" ] || { echo "FAIL the pinned runtime tarball is linux-x64; use RUNTIME_SOURCE=installed on this machine"; return 1; }
      tarball=${RUNTIME_TARBALL:-$work/dotnet-runtime-$runtime_pinned-linux-x64.tar.gz}
      if [ -z "${RUNTIME_TARBALL:-}" ]; then
        echo "download: $runtime_url"
        curl -fsSL --retry 3 -o "$tarball" "$runtime_url" || { echo "FAIL the download failed"; return 1; }
      fi
      sh "$here/extract-runtime.sh" "$tarball" "$runtime_sha512" "$work/runtime" || return 1
      rt_dotnet=$work/runtime/dotnet
      droot=$work/runtime
      ;;
    installed)
      echo "the installed runtime is used: $dotnet_path"
      ;;
    *)
      echo "FAIL RUNTIME_SOURCE is pinned or installed, not '$runtime_source'"
      return 1
      ;;
  esac
  "$rt_dotnet" --list-runtimes || return 1
  want="Microsoft.NETCore.App $runtime_pinned "
  [ "$("$rt_dotnet" --list-runtimes | grep -c '^Microsoft.NETCore.App ')" -eq 1 ] || { echo "FAIL more than one Microsoft.NETCore.App runtime is visible to the program's dotnet"; return 1; }
  "$rt_dotnet" --list-runtimes | grep -q "^$want" || { echo "FAIL the runtime the program runs on is not Microsoft.NETCore.App $runtime_pinned"; return 1; }
  echo "ok   the program runs on Microsoft.NETCore.App $runtime_pinned, DOTNET_ROOT=$droot"
}

# publish_copy <copy dir> <publish dir> [old]: a copy of the sources (not the build output), restored and published. With
# `old` the copied files carry the time 2000-01-01.
publish_copy() {
  mkdir -p "$1" || return 1
  cp "$dnroot"/src/*.cs "$dnroot"/src/*.csproj "$1/" || return 1
  # The pin travels with the copy: the SDK that builds it is the one in global.json, whatever else the machine has installed.
  cp "$repo/global.json" "$1/" || return 1
  in_copy=$(cd "$1" && "$dotnet" --version) || { echo "FAIL dotnet cannot run in the copy of the sources (is the pinned SDK $sdk_pinned installed?)"; return 1; }
  [ "$in_copy" = "$sdk_pinned" ] || { echo "FAIL the SDK that builds the copy is $in_copy, not the pinned $sdk_pinned"; return 1; }
  if [ "${3:-}" = old ]; then
    touch -t 200001010000 "$1"/*.cs "$1"/*.csproj || return 1
  fi
  "$dotnet" restore "$1/LangAtProbe.csproj" --nologo -v q || return 1
  # A BCL-only program restores no library.
  grep -Eq '"libraries": \{\}' "$1/obj/project.assets.json" || { echo "FAIL the restore resolved a library: the probe is not BCL only"; return 1; }
  # shellcheck disable=SC2086  # the flags are separate words on purpose
  "$dotnet" publish "$1/LangAtProbe.csproj" $publish_flags -o "$2"
}

build_twice() {
  echo "The two builds use copies of the sources in different directories, and the second copy's files carry the time 2000-01-01,"
  echo "so neither the path nor the file times may reach the output."
  rm -rf "$one" "$two" "$pub_one" "$pub_two"
  publish_copy "$one" "$pub_one" || return 1
  publish_copy "$two" "$pub_two" old || return 1
  echo "--- the publish directory"
  (cd "$pub_one" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  listed=$(cd "$pub_one" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | tr '\n' ' ')
  wanted=$(printf '%s\n' $files | LC_ALL=C sort | tr '\n' ' ')
  [ "$listed" = "$wanted" ] || { echo "FAIL the publish directory does not hold exactly: $files"; return 1; }
  echo "ok   the publish directory holds exactly the three files"
  all_equal=1
  for f in $files; do
    first=$(hash_file "$pub_one/$f")
    second=$(hash_file "$pub_two/$f")
    equal_hashes "$f" "$first" "$second" || all_equal=0
  done
  [ "$all_equal" -eq 1 ] || return 1
  echo "tree hash, first build:  $(tree_hash "$pub_one")"
  echo "tree hash, second build: $(tree_hash "$pub_two")"
  echo "--- LangAtProbe.runtimeconfig.json"
  cat "$pub_one/LangAtProbe.runtimeconfig.json"
  echo "--- LangAtProbe.deps.json"
  cat "$pub_one/LangAtProbe.deps.json"
}

# The probe reads no seed, /proc, CPU count or environment, and no clock but the stopwatch counter (its line carries the
# monotonic counter difference between the start and the end of the run: vtime_ns).
forbidden_reads() {
  # The allowance for Stopwatch.GetTimestamp applies to Clock.cs only: a clock read in any other file fails here.
  sh "$here/check-sources-dotnet.sh" --allow-stopwatch-in Clock.cs "$dnroot/src" || return 1
  # This makes sure Clock.cs has exactly one read, and no renamed import.
  sh "$here/check-stopwatch-use.sh" "$dnroot/src/Clock.cs" || return 1
  echo "ok   the probe reads none of them; its only clock read is the one Stopwatch.GetTimestamp (vtime_ns)"
  grep -n 'Stopwatch' "$dnroot"/src/*.cs
}

# The environment profiles of the experiment: the exact variables, as the test pins them.
profiles() {
  echo "--- profiles.manifest"
  cat "$dnroot/profiles.manifest"
  for arm in a b; do
    echo "--- arm $arm, with <runtime> standing for the directory of the dotnet executable"
    sh "$dnroot/profile-env.sh" "$arm" "<runtime>" || return 1
  done
}

# B4, format only (test_probe.sh): every run exits 0 with an empty stderr and one stdout line of the six-field form, any
# argument is a usage error (64), an unwritable stdout exits 3, and both profiles run and print a valid line. The distinct
# counts are information. Under the engine only --sched-seed is passed to Hermit, whose --rng-seed defaults from --seed,
# which nobody passes: guest randomness is the same for every trial and seed. Same-seed equality still detects an entropy
# leak, and a difference across seeds would itself signal one.
native() {
  sh "$dnroot/test_probe.sh" "$rt_dotnet" "$dll" "$runs"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace; dup and fcntl are traced to see which descriptor is a duplicate of descriptor 1. One run per
# profile arm. The runtime duplicates descriptor 1 for its console stream, so the line is written to the duplicate:
# check-probe-writes.sh --dup-of-stdout requires exactly one whole write of it, on a descriptor a traced dup of descriptor 1
# created, and no write at all to descriptor 1 or 2. That the duplicate is the program's standard output is shown here by
# the captured files: stdout holds exactly the line and nothing else, and stderr is empty.
events() {
  for arm in a b; do
    words=$(sh "$dnroot/profile-env.sh" "$arm" "$droot") || return 1
    rm -f "$work"/strace-"$arm".*
    # The variable words are separate arguments to env on purpose.
    # shellcheck disable=SC2086
    env -i PATH="$PATH" $words strace -ff -s 4096 -e trace=write,writev,fcntl,dup -o "$work/strace-$arm" "$rt_dotnet" "$dll" >"$work/stdout-$arm.txt" 2>"$work/stderr-$arm.txt" || return 1
    echo "--- arm $arm: $(ls "$work"/strace-"$arm".* | wc -l | tr -d ' ') trace files"
    sh "$here/check-probe-writes.sh" --dup-of-stdout "$work"/strace-"$arm".* || return 1
    grep -h 'write(' "$work"/strace-"$arm".* | grep 'PROBE v1' | cut -c1-80
    [ "$(wc -l <"$work/stdout-$arm.txt" | tr -d ' ')" -eq 1 ] || { echo "FAIL arm $arm: stdout is not exactly one line"; cat "$work/stdout-$arm.txt"; return 1; }
    grep -Eq "$line_rx" "$work/stdout-$arm.txt" || { echo "FAIL arm $arm: stdout is not exactly the six-field line"; cat "$work/stdout-$arm.txt"; return 1; }
    [ ! -s "$work/stderr-$arm.txt" ] || { echo "FAIL arm $arm: something on stderr:"; cat "$work/stderr-$arm.txt"; return 1; }
    echo "ok   arm $arm: stdout is exactly the one line, stderr is empty"
  done
}

# The probe's own unit checks, on a clean copy of the sources (the build output of the checkout is not used).
unit_checks() {
  copy=$work/unit
  rm -rf "$copy"
  mkdir -p "$copy/src" "$copy/selftest" || return 1
  cp "$dnroot"/src/*.cs "$dnroot"/src/*.csproj "$copy/src/" || return 1
  cp "$dnroot"/selftest/*.cs "$dnroot"/selftest/*.csproj "$copy/selftest/" || return 1
  "$dotnet" run --project "$copy/selftest/SelfTest.csproj" --nologo -v q
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-probe-dotnet.sh."
artifact_name=$artifact
artifact_sha_label="tree hash (the sha256 of the sorted per-file sha256 lines of B1)"
artifact_sha="<not built>"
record_no_checklist=1

say "Scope: this record covers probe-dotnet, a BCL-only program (no SDK, no property) used to check scheduling behaviour of .NET"
say "threads. It prints one \`PROBE v1\` line of six fields. Per the checks that apply to a probe, B1 and B4 (format only: the"
say "line matches the six-field form and the exit status is 0) are required; B5 is run as well. B2, B3, B6 and B7 belong to the"
say "AT programs and do not apply."
say ""
say "The artifact is three files, framework-dependent: LangAtProbe.dll, LangAtProbe.runtimeconfig.json and LangAtProbe.deps.json."
say "It is run as \`dotnet LangAtProbe.dll\` with the environment of a profile (E1). The artifact line below is the tree hash of"
say "the three files; SHA256SUMS in the uploaded artifact lists each file."
say ""
say "Runtime: the SDK only builds. B4 and B5 run the program on the pinned runtime (step R1): the tarball, checked against its"
say "sha512 and extracted, is DOTNET_ROOT for every run; R1 fails unless the runtime that runs the program is exactly the pinned"
say "version and the only Microsoft.NETCore.App visible to it."
say ""
say "Preflight on the target machine (read-only, not run here): the runtime needs ICU. Check that libicu is installed, for example"
say "\`ls /usr/lib/x86_64-linux-gnu/libicuuc.so.74*\`. Without ICU the runtime aborts at startup, and the probe would read as a crash."
say "DOTNET_SYSTEM_GLOBALIZATION_INVARIANT is deliberately not set in either profile."
say ""
say "Randomness: \`rand\` is \`Random.Shared.NextInt64()\`, the runtime's default random source, as an unsigned number. That call"
say "returns a non-negative 63-bit value, so the top bit of the 16-hex-digit field is always 0 (the specification's table names this"
say "call for .NET and calls the field 64 bits: a documented question). Under the engine only --sched-seed"
say "reaches Hermit, whose --rng-seed defaults from --seed, which is not passed, so guest randomness is the same for every trial"
say "and seed. Same-seed equality still detects an entropy leak, and a difference across seeds would itself signal one."
say ""
say "Reproducibility: the project sets Deterministic and ContinuousIntegrationBuild, writes no debug symbols and no source"
say "revision into the assembly, and builds without an apphost. B1 builds from two copies of the sources in different"
say "directories with different file times. The pinned artifact comes from one named producer: the one named above."
say ""
step B1 "SDK, build command and project" versions
step R1 "the runtime the program runs on: the pinned version, from the checked tarball" pinned_runtime
step B1 "the publish, twice, with equal sha256 of every file" build_twice
if [ -f "$pub_one/LangAtProbe.dll" ]; then
  artifact_sha=$(publish_tree "$pub_one" "$artifact" "$(tree_hash "$pub_one")") || artifact_sha="<not built>"
fi
not_applicable B2 "disassembly of the busy loop and the window" "Not applicable: the probe has no fixture window to prove. Its unlocked counter exists to give the scheduler something to interleave, and its value is not part of the line."
not_applicable B3 "seccomp self-test" "Not applicable: the probe has no no-syscall window."
step F1 "forbidden reads: no seed, /proc, /sys, CPU count, environment or clock in the probe's sources (Stopwatch.GetTimestamp allowed, once, in Clock.cs)" forbidden_reads
step E1 "environment profiles: the exact variables of arm a and arm b" profiles
step B4 "$runs native runs: the line matches the six-field form and the exit status is 0; any argument is a usage error; both profiles run" native
if command -v strace >/dev/null 2>&1; then
  step B5 "under each profile the result line is one whole write to a duplicate of descriptor 1, nothing is written to descriptors 1 or 2, stdout is exactly the line and stderr is empty, and no protocol line exists" events
else
  not_run B5 "the result line is one whole write" "strace is not installed here."
fi
not_applicable B6 "the SDK's conformance" "Not applicable: the probe does not use the SDK."
not_applicable B7 "wide differs from buggy only in the loop count" "Not applicable: the probe has no modes."
step T1 "the probe's own unit checks (selftest, on a clean copy of the sources)" unit_checks

finish_record
exit $failed
