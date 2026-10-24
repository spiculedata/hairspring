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
# Environment: DOTNET is the dotnet executable (default dotnet), SDK_PINNED and RUNTIME_PINNED the pinned versions (defaults
# 10.0.402 and 10.0.13), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARTIFACT_DIR receives a
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
sdk_pinned=${SDK_PINNED:-10.0.402}
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
# The runtime directory: the one that holds the dotnet executable (resolved through links).
dotnet_path=$(command -v "$dotnet" 2>/dev/null || echo "$dotnet")
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
  "$dotnet" --list-runtimes | grep -q "^Microsoft.NETCore.App $runtime_pinned " || { echo "FAIL there is no Microsoft.NETCore.App $runtime_pinned runtime"; return 1; }
  echo "ok   the runtime is the pinned $runtime_pinned"
  echo "restore command: dotnet restore LangAtProbe.csproj --nologo -v q"
  echo "build command: dotnet publish LangAtProbe.csproj $publish_flags -o <dir>"
  echo "--- the project file (framework-dependent, no packages, no apphost, no debug symbols)"
  cat "$dnroot/src/LangAtProbe.csproj"
  [ "$(grep -c PackageReference "$dnroot/src/LangAtProbe.csproj")" -eq 0 ] || { echo "FAIL the project references packages"; return 1; }
  echo "ok   the project has no PackageReference (BCL only)"
}

# publish_copy <copy dir> <publish dir> [old]: a copy of the sources (not the build output), restored and published. With
# `old` the copied files carry the time 2000-01-01.
publish_copy() {
  mkdir -p "$1" || return 1
  cp "$dnroot"/src/*.cs "$dnroot"/src/*.csproj "$1/" || return 1
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
  sh "$here/check-sources-dotnet.sh" --allow-stopwatch "$dnroot/src" || return 1
  # --allow-stopwatch lets Stopwatch.GetTimestamp through; this makes sure there is exactly one, and no renamed import.
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
  sh "$dnroot/test_probe.sh" "$dotnet_path" "$dll" "$runs"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace. One run per profile arm.
events() {
  for arm in a b; do
    words=$(sh "$dnroot/profile-env.sh" "$arm" "$droot") || return 1
    rm -f "$work"/strace-"$arm".*
    # The variable words are separate arguments to env on purpose.
    # shellcheck disable=SC2086
    env -i PATH="$PATH" $words strace -ff -s 4096 -e trace=write,writev -o "$work/strace-$arm" "$dotnet_path" "$dll" >"$work/stdout-$arm.txt" 2>"$work/stderr-$arm.txt" || return 1
    echo "--- arm $arm: $(ls "$work"/strace-"$arm".* | wc -l | tr -d ' ') trace files"
    # The runtime duplicates descriptor 1 for its console stream, so the line is on another descriptor (--any-fd): exactly
    # one write of it, whole, wherever it goes. That it reaches stdout is B4's job (the line is read from stdout there).
    sh "$here/check-probe-writes.sh" --any-fd "$work"/strace-"$arm".* || return 1
    grep -h 'write(' "$work"/strace-"$arm".* | grep 'PROBE v1' | cut -c1-80
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
say "Randomness: \`rand\` is 64 bits from Random.Shared, the runtime's default random source. Under the engine only --sched-seed"
say "reaches Hermit, whose --rng-seed defaults from --seed, which is not passed, so guest randomness is the same for every trial"
say "and seed. Same-seed equality still detects an entropy leak, and a difference across seeds would itself signal one."
say ""
say "Reproducibility: the project sets Deterministic and ContinuousIntegrationBuild, writes no debug symbols and no source"
say "revision into the assembly, and builds without an apphost. B1 builds from two copies of the sources in different"
say "directories with different file times. The pinned artifact comes from one named producer: the one named above."
say ""
step B1 "SDK, runtime, build command, project and two publishes with equal sha256 of every file" versions
step B1 "the publish, twice" build_twice
if [ -f "$pub_one/LangAtProbe.dll" ]; then
  artifact_sha=$(publish_tree "$pub_one" "$artifact" "$(tree_hash "$pub_one")") || artifact_sha="<not built>"
fi
not_applicable B2 "disassembly of the busy loop and the window" "Not applicable: the probe has no fixture window to prove. Its unlocked counter exists to give the scheduler something to interleave, and its value is not part of the line."
not_applicable B3 "seccomp self-test" "Not applicable: the probe has no no-syscall window."
step F1 "forbidden reads: no seed, /proc, CPU count, environment or clock in the probe's sources (Stopwatch.GetTimestamp allowed, once)" forbidden_reads
step E1 "environment profiles: the exact variables of arm a and arm b" profiles
step B4 "$runs native runs: the line matches the six-field form and the exit status is 0; any argument is a usage error; both profiles run" native
if command -v strace >/dev/null 2>&1; then
  step B5 "the result line is exactly one write to stdout under each profile, and no protocol line exists" events
else
  not_run B5 "the result line is exactly one write" "strace is not installed here."
fi
not_applicable B6 "the SDK's conformance" "Not applicable: the probe does not use the SDK."
not_applicable B7 "wide differs from buggy only in the loop count" "Not applicable: the probe has no modes."
step T1 "the probe's own unit checks (selftest, on a clean copy of the sources)" unit_checks

finish_record
exit $failed
