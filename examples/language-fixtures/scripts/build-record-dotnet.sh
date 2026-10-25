#!/bin/sh
# Generates the BUILD-RECORD of the .NET fixture artifact fixture-dotnet (LangAt.dll with its runtimeconfig.json and deps.json,
# and Hairspring.dll, the .NET SDK, beside it): checks B1 to B7 of the fixture contract, in one Markdown file.
#
#   build-record-dotnet.sh [out-file]
#
# Run it on Linux with the pinned .NET SDK (the manual CI job does). It publishes the fixture twice (two copies of the sources
# in different directories, with different file times) and requires equal sha256 for the four files (B1), searches the
# sources for forbidden reads, runs the program natively on the pinned runtime in the default environment and under each
# environment profile arm (B4, with a negative control), runs strace -ff on one run per arm and without a profile to check that
# every protocol line and the result line is one write (B5), runs the SDK's own tests and conformance (B6) and prints the
# source of the modes (B7). It then captures the evidence for the rest of the alignment checklist from the built artifact and
# ticks each checklist line only when the evidence that proves it passed (record-lib.sh). Nothing here runs Hermit or touches the
# box. Exits 1 if any step failed.
#
# Not checked, and said so in the record: the machine code the JIT makes of the two worker loops (B2), and that the loops are
# never recompiled. The SDK (DOTNET) only builds. The program is run (B4, B5) on the pinned runtime: by default the runtime
# tarball dotnet-runtime-<RUNTIME_PINNED>-linux-x64.tar.gz, fetched from RUNTIME_URL (or taken from RUNTIME_TARBALL), checked
# against RUNTIME_SHA512 and extracted under a temporary directory, which is DOTNET_ROOT for every run, so the record cannot
# silently run on a newer 10.0.x patch that the machine has installed. RUNTIME_SOURCE=installed runs on the installed runtime
# instead (for a machine that is not x86-64 Linux); the record says which was used, and still requires the version to be pinned.
#
# Environment: DOTNET is the dotnet executable (default dotnet), SDK_PINNED and RUNTIME_PINNED the pinned versions (defaults
# 10.0.402 and 10.0.13), IMAGE_DIGEST is copied into B1, RUNS is the native run count (default 50), ARTIFACT_DIR receives a
# copy of the pinned files and a SHA256SUMS file. The producer fields come from the GitHub Actions environment (record-lib.sh).

# The check functions below are run through step, which shellcheck cannot see. The record_*, artifact_*, evidence_* and
# barrier_sources variables are the interface with record-lib.sh, which reads them (and sets failed).
# shellcheck disable=SC2329,SC2034,SC2154
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
repo=$(cd "$root/../.." && pwd)
dnroot=$root/dotnet
sdkdir=$repo/sdks/dotnet
runs=${RUNS:-50}
dotnet=${DOTNET:-dotnet}
sdk_pinned=${SDK_PINNED:-10.0.402}
runtime_pinned=${RUNTIME_PINNED:-10.0.13}
out=${1:-$root/build/record-dotnet.md}
mkdir -p "$(dirname "$out")" || exit 1
# shellcheck source=record-lib.sh
. "$here/record-lib.sh"

artifact="fixture-dotnet"
files="LangAt.dll LangAt.runtimeconfig.json LangAt.deps.json Hairspring.dll"
lang=dotnet
work=$(mktemp -d "${TMPDIR:-/tmp}/record-work.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
record_init
one=$work/build-one-for-the-first-publish
two=$work/second/build-two-for-the-second-publish
pub_one=$work/publish-one
pub_two=$work/publish-two
dll=$pub_one/LangAt.dll
dotnet_path=$(command -v "$dotnet" 2>/dev/null || echo "$dotnet")
runtime_source=${RUNTIME_SOURCE:-pinned}
# The tarball of the pinned runtime and its sha512 (published by Microsoft).
runtime_url=${RUNTIME_URL:-https://builds.dotnet.microsoft.com/dotnet/Runtime/$runtime_pinned/dotnet-runtime-$runtime_pinned-linux-x64.tar.gz}
runtime_sha512=${RUNTIME_SHA512:-9574d95364110409416735e7842796f8d614c21d74cb25d204a3e4cfd11b2c05efdf19ca89b8069650a04f64add2705967dcb04b8665a36d7a3f30a5d8c05afe}
# The dotnet executable the program runs on, and its runtime directory (DOTNET_ROOT): set by the runtime step (R1); until
# then the installed one.
rt_dotnet=$dotnet_path
droot=$(dirname "$(readlink -f "$dotnet_path" 2>/dev/null || echo "$dotnet_path")")
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
  echo "restore command: dotnet restore LangAt.csproj --nologo -v q"
  echo "build command: dotnet publish LangAt.csproj $publish_flags -o <dir>"
  echo "--- the project file (framework-dependent, no packages, no apphost, no debug symbols)"
  cat "$dnroot/at/LangAt.csproj"
  echo "--- the SDK's project file"
  cat "$sdkdir/src/Hairspring.csproj"
  if grep -q PackageReference "$dnroot/at/LangAt.csproj" "$sdkdir/src/Hairspring.csproj"; then
    echo "FAIL a project references packages"
    return 1
  fi
  echo "ok   the projects have no PackageReference (BCL only; the one reference is the SDK's project)"
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

# publish_copy <copy dir> <publish dir> [old]: a copy of the sources (not the build output) in the same relative layout, restored
# and published. With `old` the copied files carry the time 2000-01-01.
publish_copy() {
  fx=$1/examples/language-fixtures/dotnet/at
  sdk=$1/sdks/dotnet/src
  mkdir -p "$fx/src" "$sdk" || return 1
  cp "$dnroot"/at/LangAt.csproj "$fx/" || return 1
  cp "$dnroot"/at/src/*.cs "$fx/src/" || return 1
  cp "$sdkdir"/src/*.cs "$sdkdir"/src/*.csproj "$sdk/" || return 1
  if [ "${3:-}" = old ]; then
    touch -t 200001010000 "$fx"/LangAt.csproj "$fx"/src/*.cs "$sdk"/*.cs "$sdk"/*.csproj || return 1
  fi
  "$dotnet" restore "$fx/LangAt.csproj" --nologo -v q || return 1
  # A BCL-only program restores no package: the only library is the SDK's own project.
  if grep -q '"type": "package"' "$fx/obj/project.assets.json"; then
    echo "FAIL the restore resolved a package: the program is not BCL only"
    return 1
  fi
  # shellcheck disable=SC2086  # the flags are separate words on purpose
  "$dotnet" publish "$fx/LangAt.csproj" $publish_flags -o "$2"
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
  echo "ok   the publish directory holds exactly the four files"
  all_equal=1
  for f in $files; do
    first=$(hash_file "$pub_one/$f")
    second=$(hash_file "$pub_two/$f")
    equal_hashes "$f" "$first" "$second" || all_equal=0
  done
  [ "$all_equal" -eq 1 ] || return 1
  echo "tree hash, first build:  $(tree_hash "$pub_one")"
  echo "tree hash, second build: $(tree_hash "$pub_two")"
  echo "--- LangAt.runtimeconfig.json"
  cat "$pub_one/LangAt.runtimeconfig.json"
  echo "--- LangAt.deps.json"
  cat "$pub_one/LangAt.deps.json"
}

forbidden_reads() {
  sh "$here/check-sources-dotnet.sh" "$dnroot/at/src" || return 1
  echo "ok   the fixture reads no seed, /proc, environment, CPU count or clock (the SDK, a library, reads its own HAIRSPRING_* variables and its own process's auxiliary vector)"
}

# The environment profiles: the exact variables, as the probe's test pins them.
profiles() {
  echo "--- profiles.manifest"
  cat "$dnroot/profiles.manifest"
  for arm in a b; do
    echo "--- arm $arm, with <runtime> standing for the directory of the dotnet executable"
    sh "$dnroot/profile-env.sh" "$arm" "<runtime>" || return 1
  done
}

# The program as the self-checks run it: a command that takes the mode as its argument, on the pinned runtime.
make_wrapper() {
  printf '#!/bin/sh\nexec env DOTNET_ROOT="%s" "%s" "%s" "$@"\n' "$droot" "$rt_dotnet" "$dll" >"$work/at-dotnet"
  chmod +x "$work/at-dotnet"
}

# B4: the whole contract natively, 50 runs of buggy and fixed, wide as a contract check, a negative control. First in the default
# environment (DOTNET_ROOT only), as a person would run it, then once under each profile arm's variables.
native() {
  make_wrapper
  echo "--- default environment (DOTNET_ROOT only)"
  LANG_AT_REQUIRE_PARALLEL=1 sh "$here/run-checks.sh" "$lang" "$work/at-dotnet" "$runs" || return 1
  LANG_AT_REQUIRE_PARALLEL=1 sh "$here/run-dotnet-profile-checks.sh" "$dnroot/profiles.manifest" "$dll" "$rt_dotnet" "$runs"
}

# -ff writes one trace file per process or thread, so nothing interleaves; they are joined before the check. -s 4096 so the
# whole line is in the trace. One run without a profile and one per profile arm. The SDK writes each protocol line to descriptor 2
# with one write(2), the fixture writes the result line to descriptor 1 with one.
events() {
  for arm in default a b; do
    if [ "$arm" = default ]; then
      words="DOTNET_ROOT=$droot"
    else
      words=$(sh "$dnroot/profile-env.sh" "$arm" "$droot") || return 1
    fi
    echo "--- arm $arm: $(printf '%s' "$words" | tr '\n' ' ')"
    rm -f "$work"/strace.*
    # The variable words are separate arguments to env on purpose.
    # shellcheck disable=SC2086
    env $words strace -ff -s 4096 -e trace=write,writev -o "$work/strace" "$rt_dotnet" "$dll" buggy >"$work/stdout.txt" 2>"$work/stderr.txt" </dev/null || return 1
    cat "$work"/strace.* >"$work/strace-all.txt"
    lines=$(grep -c '^HAIRSPRING_EVENT' "$work/stderr.txt")
    sh "$here/check-writes.sh" "$work/strace-all.txt" >"$work/writes.txt" || { cat "$work/writes.txt"; return 1; }
    cat "$work/writes.txt"
    written=$(cut -d' ' -f1 "$work/writes.txt")
    echo "HAIRSPRING_EVENT lines on stderr: $lines; protocol line writes in the trace: $written"
    [ "$lines" -gt 0 ] && [ "$lines" -eq "$written" ] || return 1
  done
}

# The SDK's own tests (the self-test and the sample programs as real processes) and the conformance tests against the
# reference parser, both with the SDK and runtime this record names.
goldens() {
  make -C "$sdkdir" test BUILD="$work/sdk-build" DOTNET="$dotnet" || return 1
  HS_REQUIRE_DOTNET=1 HS_DOTNET=$dotnet cargo test --manifest-path "$repo/Cargo.toml" -p hairspring-props --test dotnet_sdk
}

# The unit tests of the fixture's own classes.
fixture_tests() {
  make -C "$dnroot" at-selftest DOTNET="$dotnet"
}

modes() {
  {
    sed -n '/public static readonly Mode Buggy/,/private static readonly Mode\[\] All/p' "$dnroot/at/src/Mode.cs"
    sed -n '/private void RacyIterations/,/^    }/p;/private void LockedIterations/,/^    }/p' "$dnroot/at/src/Race.cs"
  } | require_lines
}

SOURCE_COMMIT=${SOURCE_COMMIT:-$(source_commit_of "$repo")}
record_title="BUILD-RECORD: $artifact"
record_generated_by="Generated by scripts/build-record-dotnet.sh."
artifact_name=$artifact
artifact_sha_label="tree hash (the sha256 of the sorted per-file sha256 lines of B1)"
artifact_sha="<not built>"
evidence_artifact=$work/at-dotnet
barrier_sources=$dnroot/at/src/Race.cs
constant_sources=$dnroot/at/src/Contract.cs

say "Scope: this record covers the fixture \`LangAt.dll\` built from this checkout (class \`LangAt.Program\`, modes buggy, fixed, wide),"
say "with its runtimeconfig.json and deps.json, and \`Hairspring.dll\`, the .NET SDK, built from sdks/dotnet and published beside it."
say "The artifact is those four files, framework-dependent, run as \`dotnet LangAt.dll <mode>\` with the environment of a profile (E1)."
say "The artifact line below is the tree hash of the four files; SHA256SUMS in the uploaded artifact lists each file. The BCL-only"
say "probe program is a different artifact with its own record (build-record-probe-dotnet.sh)."
say ""
say "Runtime: the SDK only builds. B4 and B5 run the program on the pinned runtime (step R1): the tarball, checked against its"
say "sha512 and extracted, is DOTNET_ROOT for every run; R1 fails unless the runtime that runs the program is exactly the pinned"
say "version and the only Microsoft.NETCore.App visible to it."
say ""
say "Preflight on the target machine (read-only, not run here): the runtime needs ICU. Check that libicu is installed, for example"
say "\`ls /usr/lib/x86_64-linux-gnu/libicuuc.so.74*\`. Without ICU the runtime aborts at startup, and the program would read as a crash."
say "DOTNET_SYSTEM_GLOBALIZATION_INVARIANT is deliberately not set in either profile."
say ""
say "Randomness: \`rand\` is \`Random.Shared.NextInt64()\`, the runtime's default random source, as an unsigned number. That call"
say "returns a non-negative 63-bit value, so the top bit of the 16-hex-digit field is always 0. Under the engine only --sched-seed"
say "reaches Hermit, whose --rng-seed defaults from --seed, which is not passed, so guest randomness is the same for every trial"
say "and seed. Same-seed equality still detects an entropy leak, and a difference across seeds would itself signal one."
say ""
say "Reproducibility: the projects set Deterministic and ContinuousIntegrationBuild, write no debug symbols and no source revision into"
say "the assemblies, and build without an apphost. B1 builds from two copies of the sources in different directories with"
say "different file times. The pinned artifact comes from one named producer: the one named above."
say ""
say "The JIT and the window (read this before B2): the window is a load of a plain field, a call of \`Spin\` (marked NoInlining) and a"
say "store of that field. The design relies on a compiler having to assume that a call it did not inline can write the field. B2 does"
say "not rely on that: it runs every mode in the default environment and under each profile arm with the JIT's own disassembly turned"
say "on (DOTNET_JitDisasm, DOTNET_JitDisasmSummary, DOTNET_JitStdOutFile), lists every compilation of the worker method with its"
say "tier, and requires each listing to show the load of the counter field, exactly one call (of Spin) and the store of the same"
say "field. It reads the compilations of this run only: a method recompiled after the process ends, or a different tier in another"
say "run, is not covered. The native runs (B4) show the race firing."
say ""
step B1 "SDK, build command and projects" versions
step R1 "the runtime the program runs on: the pinned version, from the checked tarball" pinned_runtime
step B1 "the publish, twice, with equal sha256 of every file" build_twice
if [ -f "$pub_one/LangAt.dll" ]; then
  artifact_sha=$(publish_tree "$pub_one" "$artifact" "$(tree_hash "$pub_one")") || artifact_sha="<not built>"
fi
jit_window() {
  sh "$here/check-dotnet-jit.sh" "$dnroot/profiles.manifest" "$dll" "$rt_dotnet"
}
step B2 "the window in the JIT's listing of the worker loops, in the default environment and under each arm" jit_window
not_applicable B3 "seccomp self-test" "Not applicable to .NET: the runtime makes system calls on other threads (garbage collector, tiered compilation, signal handling), so a filter that allows only exit would kill it. The window is not free of other activity, and this record does not say it is: \`Spin\` is called inside the window, and a thread can be stopped at a garbage-collection safepoint in it. That only makes the race easier to reach: it gives the other workers more time between the load and the store. What it means for the signature of the race is for the reviewer to say; this record does not." yes
step F1 "forbidden reads: no seed, /proc, environment, CPU count or clock in the fixture's sources" forbidden_reads
step E1 "environment profiles: the exact variables of arm a and arm b" profiles
step B4 "$runs native runs of buggy and fixed, wide as a contract check, a negative control, in the default environment and under each profile arm (at least two CPUs required)" native
if command -v strace >/dev/null 2>&1; then
  step B5 "each HAIRSPRING_EVENT line is one write(2) to descriptor 2, and the result line is exactly one write to descriptor 1, without a profile and under each arm" events
else
  not_run B5 "each HAIRSPRING_EVENT line is one write(2)" "strace is not installed here."
fi
if ! command -v cargo >/dev/null 2>&1; then
  not_run B6 "the SDK's conformance" "cargo is not installed here. Run \`HS_REQUIRE_DOTNET=1 cargo test -p hairspring-props --test dotnet_sdk\` on a host with cargo and paste its output."
else
  step B6 "the SDK's own tests and conformance against the reference parser" goldens
fi
step B7 "the modes: wide differs from buggy only in the busy loop length" modes
step T1 "the fixture's own tests" fixture_tests

say "## Evidence for the other checklist lines (captured from the built artifact above, run as \`dotnet LangAt.dll\` on the pinned runtime)"
say ""
make_wrapper
evidence_steps

alignment_checklist \
  "One artifact, named \`$artifact\` (\`LangAt.dll\` with the SDK beside it); no separate racy and locked builds, no flag choosing the mode." \
  "argv[1] is buggy, fixed or wide; anything else, or nothing, prints a usage line on stderr and exits 64." \
  "No read of the seed, /proc, the CPU count, the environment or the clock in the fixture's sources outside the runtime and the random source (the SDK, a library, reads its own HAIRSPRING_* variables and its own auxiliary vector)."

finish_record
exit $failed
