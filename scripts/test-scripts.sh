#!/usr/bin/env bash
# Tests for scripts/install-hermit.sh and scripts/install.sh, with fake commands and fixture /proc and /sys trees.
#
#   bash scripts/test-scripts.sh               run every test
#   bash scripts/test-scripts.sh --mutations   also prove the tests can fail: each mutant of a script must be caught
#
# Nothing here touches the real machine: every test runs the script under a sandbox whose PATH holds only fake
# commands (uname, id, git, cargo, rustup, pkg-config, sudo, sysctl, ...) and symlinks to a short list of real tools.
# `sudo` and `sysctl` are tripwires: any call to them is recorded and fails the test. Hermit and the hairspring CLI are
# small shell scripts that print what the real ones print.
#
# Environment: HS_SCRIPTS_DIR runs the tests against another copy of the scripts (the mutation run uses this).
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SCRIPTS_DIR="${HS_SCRIPTS_DIR:-$HERE}"
REAL_PATCH="$REPO/examples/glibc-condlock/experiment/patches/hermit-25781c5-replay-backport.diff"
readonly PINNED_COMMIT="25781c5c48d47c316324fd912b7dcafa394c339a"
readonly PINNED_TOOLCHAIN="nightly-2026-07-12"
# Real tools the scripts under test may use. Anything else (git, cargo, uname, id, ...) is faked or absent.
readonly REAL_TOOLS="bash sh env cat grep head tail cut tr sed awk sort uniq wc mkdir rm cp mv ln chmod ls dirname basename
mktemp install date df getconf touch sleep find cmp tee xargs sha256sum shasum curl cksum rmdir readlink seq"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-script-tests.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0
CURRENT=""

# ---- assertions -------------------------------------------------------------------------------------------------
fail_now() {
  FAIL=$((FAIL + 1))
  printf 'FAIL %s: %s\n' "$CURRENT" "$1"
  [ -z "${2:-}" ] || printf '%s\n' "$2" | sed 's/^/    | /'
  TEST_FAILED=1
}

assert_rc() {
  [ "$RC" = "$1" ] || fail_now "exit code $RC, expected $1" "stdout: $(cat "$SB/out")
stderr: $(cat "$SB/err")"
}
assert_out_has() { grep -qF -- "$1" "$SB/out" || fail_now "stdout lacks: $1" "$(cat "$SB/out")"; }
assert_out_lacks() { ! grep -qF -- "$1" "$SB/out" || fail_now "stdout has: $1" "$(cat "$SB/out")"; }
assert_err_has() { grep -qF -- "$1" "$SB/err" || fail_now "stderr lacks: $1" "$(cat "$SB/err")"; }
assert_log_has() { grep -qF -- "$1" "$SB/calls.log" || fail_now "no call matching: $1" "$(cat "$SB/calls.log")"; }
assert_log_lacks() { ! grep -qF -- "$1" "$SB/calls.log" || fail_now "unexpected call: $1" "$(cat "$SB/calls.log")"; }
assert_exists() { [ -e "$1" ] || fail_now "missing: $1"; }
assert_absent() { [ ! -e "$1" ] || fail_now "should not exist: $1"; }
assert_count() {
  # $1 file, $2 fixed string, $3 expected number of lines containing it
  local n
  n="$(grep -cF -- "$2" "$1" || true)"
  [ "$n" = "$3" ] || fail_now "expected $3 lines with '$2', found $n" "$(cat "$1")"
}
assert_no_tripwire() { [ ! -s "$SB/forbidden.log" ] || fail_now "sudo or sysctl was called" "$(cat "$SB/forbidden.log")"; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# ---- sandbox ----------------------------------------------------------------------------------------------------
link_real_tools() {
  mkdir -p "$TMP/real"
  local tool path
  for tool in $REAL_TOOLS; do
    path="$(command -v "$tool" 2>/dev/null || true)"
    case "$path" in /*) ln -sf "$path" "$TMP/real/$tool" ;; esac
  done
}

mk_fake() {
  local name="$1" body="$2"
  cat >"$BIN/$name" <<EOF
#!/bin/bash
echo "$name \$*" >>"\$FAKE_LOG"
$body
EOF
  chmod +x "$BIN/$name"
}

make_fake_hermit() {
  cat >"$SB/fake-hermit" <<'EOF'
#!/bin/bash
if [ -n "${FAKE_HERMIT_VERSION_FAIL:-}" ] && [ "$1" = "--version" ]; then
  echo "${FAKE_HERMIT_VERSION_FAIL}" >&2
  exit 1
fi
case "$1" in
  --version) echo "hermit 0.4.1 (2026-10-15, test build)" ;;
  --help)
    echo "Usage: hermit [OPTIONS] <COMMAND>"
    [ -n "${FAKE_HERMIT_NO_LOG:-}" ] || echo "      --log <LEVEL>  Set the logging level"
    echo "Commands:"
    echo "  run  Run a program"
    ;;
  run)
    flags="--epoch --sched-seed --summary-json --chaos --sched-heuristic --max-timeslice --skid-margin"
    for f in $flags; do
      [ "$f" = "${FAKE_HERMIT_MISSING_FLAG:-}" ] || echo "      $f"
    done
    ;;
esac
EOF
  chmod +x "$SB/fake-hermit"
}

make_fake_hairspring() {
  cat >"$SB/fake-hairspring" <<'EOF'
#!/bin/bash
case "$1" in
  --version)
    if [ -n "${FAKE_HS_VERSION_FAIL:-}" ]; then echo "boom" >&2; exit 1; fi
    echo "hairspring ${FAKE_HS_VERSION:-9.9.9}"
    ;;
  doctor)
    if [ "${2:-}" = "--process-backend" ]; then
      echo "process backend: prerequisites on this machine"
      if [ -n "${FAKE_BACKEND_OK:-}" ]; then echo "result: ok: fake"; exit 0; fi
      echo "result: CANNOT RUN the process backend on this machine:"
      exit "${FAKE_BACKEND_RC:-4}"
    fi
    echo "lock-lease: deterministic over 8 seeds x 200 steps"
    exit "${FAKE_DOCTOR_RC:-0}"
    ;;
esac
EOF
  chmod +x "$SB/fake-hairspring"
}

new_sandbox() {
  SB="$TMP/$1"
  BIN="$SB/bin"
  mkdir -p "$BIN" "$SB/home" "$SB/tmp" "$SB/state" "$SB/proc/sys/kernel/yama" "$SB/sys/bus/event_source/devices/cpu" "$SB/root"
  : >"$SB/calls.log"
  : >"$SB/forbidden.log"
  make_fake_hermit
  make_fake_hairspring
  echo 1 >"$SB/proc/sys/kernel/perf_event_paranoid"
  echo 1 >"$SB/proc/sys/kernel/yama/ptrace_scope"
  echo "flags : fpu vme" >"$SB/proc/cpuinfo"
  mk_fake uname 'case "$1" in -s) echo "${FAKE_OS:-Linux}" ;; -m) echo "${FAKE_ARCH:-x86_64}" ;; -r) echo "${FAKE_KERNEL:-6.17.0-42-generic}" ;; *) echo "${FAKE_OS:-Linux}" ;; esac'
  mk_fake id 'echo "${FAKE_UID:-1000}"'
  mk_fake sudo 'echo "sudo $*" >>"$FAKE_FORBIDDEN"; exit 99'
  mk_fake sysctl 'echo "sysctl $*" >>"$FAKE_FORBIDDEN"; exit 99'
  mk_fake cc 'exit 0'
  mk_fake pkg-config 'if [ "$1" = "--exists" ]; then case " ${FAKE_MISSING_PC:-} " in *" $2 "*) exit 1 ;; esac; fi; exit 0'
  mk_fake rustup 'case "$1 $2" in
  "toolchain list") if [ -z "${FAKE_NO_TOOLCHAIN:-}" ] || [ -e "$FAKE_STATE/toolchain-installed" ]; then echo "nightly-2026-07-12-x86_64-unknown-linux-gnu"; fi ;;
  "toolchain install") touch "$FAKE_STATE/toolchain-installed" ;;
esac'
  mk_fake git 'if [ "$1" = "-C" ]; then shift 2; fi
case "$1" in
  clone) for a in "$@"; do last="$a"; done; mkdir -p "$last"; [ -z "${FAKE_TAMPER_PATCH:-}" ] || echo tamper >>"$HAIRSPRING_HERMIT_PATCH"; printf "[toolchain]\nchannel = \"nightly-2026-07-12\"\n" >"$last/rust-toolchain.toml" ;;
  rev-parse) echo "${FAKE_GIT_HEAD:-25781c5c48d47c316324fd912b7dcafa394c339a}" ;;
  apply) if [ "$2" = "--check" ]; then exit "${FAKE_APPLY_CHECK_RC:-0}"; fi ;;
esac'
  mk_fake cargo 'echo "toolchain=${RUSTUP_TOOLCHAIN:-}" >>"$FAKE_LOG"
case " $* " in
  *" -p hermit "*) out="target/release"; src="$FAKE_HERMIT_SRC"; name=hermit ;;
  *) out="${CARGO_TARGET_DIR:-target}/release"; src="$FAKE_HAIRSPRING_SRC"; name=hairspring ;;
esac
mkdir -p "$out"; cp "$src" "$out/$name"; chmod +x "$out/$name"'
}

# Runs a command in the sandbox: no inherited environment, sandbox PATH, fixture roots. Extra VAR=value words come first.
RC=0
runx() {
  local extra=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    extra+=("$1")
    shift
  done
  shift
  RC=0
  env -i \
    PATH="$BIN:$TMP/real" HOME="$SB/home" TMPDIR="$SB/tmp" \
    FAKE_LOG="$SB/calls.log" FAKE_FORBIDDEN="$SB/forbidden.log" FAKE_STATE="$SB/state" \
    FAKE_HERMIT_SRC="$SB/fake-hermit" FAKE_HAIRSPRING_SRC="$SB/fake-hairspring" \
    HAIRSPRING_PROC_ROOT="$SB/proc" HAIRSPRING_SYS_ROOT="$SB/sys" HAIRSPRING_ROOT_DIR="$SB/root" \
    HAIRSPRING_HERMIT_PATCH="${PATCH_UNDER_TEST:-$REAL_PATCH}" CARGO_TARGET_DIR="$SB/target" \
    ${extra[@]+"${extra[@]}"} \
    "$@" >"$SB/out" 2>"$SB/err" || RC=$?
}

# Convenience: `hermit_run [VAR=val ...] -- args...`
hermit_run() {
  local extra=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    extra+=("$1")
    shift
  done
  shift
  runx ${extra[@]+"${extra[@]}"} -- bash "$SCRIPTS_DIR/install-hermit.sh" "$@"
}

installer_run() {
  local extra=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    extra+=("$1")
    shift
  done
  shift
  runx ${extra[@]+"${extra[@]}"} -- bash "$SCRIPTS_DIR/install.sh" "$@"
}

snapshot_host() {
  (cd "$SB" && find proc sys root -print | sort && find proc sys root -type f -exec cat {} + | cksum)
}

make_release() {
  # $1 directory, $2.. asset names: each gets the fake CLI; SHA256SUMS lists them
  local dir="$1" asset
  shift
  mkdir -p "$dir"
  : >"$dir/SHA256SUMS"
  for asset in "$@"; do
    cp "$SB/fake-hairspring" "$dir/$asset"
    printf '%s  %s\n' "$(sha256_of "$dir/$asset")" "$asset" >>"$dir/SHA256SUMS"
  done
}

# ---- install-hermit.sh: the read-only check ---------------------------------------------------------------------
t_hermit_help_lists_the_commands() {
  new_sandbox "$1"
  hermit_run -- --help
  assert_rc 0
  assert_out_has "install-hermit.sh check"
  assert_out_has "--prefix"
  assert_out_has "$PINNED_COMMIT"
}

t_hermit_unknown_option_is_a_usage_error() {
  new_sandbox "$1"
  hermit_run -- --frobnicate
  assert_rc 2
  assert_err_has "unknown option"
}

t_hermit_jobs_must_be_a_positive_number() {
  new_sandbox "$1"
  hermit_run -- install --jobs 0
  assert_rc 2
}

t_check_passes_on_a_healthy_host_and_changes_nothing() {
  new_sandbox "$1"
  local before after
  before="$(snapshot_host)"
  hermit_run -- check
  after="$(snapshot_host)"
  assert_rc 0
  assert_out_has "result: ok"
  assert_out_lacks "FAIL"
  assert_no_tripwire
  [ "$before" = "$after" ] || fail_now "check changed the fixture tree"
  assert_log_lacks "git "
  assert_log_lacks "cargo "
}

t_check_refuses_macos() {
  new_sandbox "$1"
  hermit_run FAKE_OS=Darwin FAKE_ARCH=arm64 -- check
  assert_rc 1
  assert_out_has "CANNOT RUN"
  assert_out_has "Linux"
}

t_check_refuses_other_architectures() {
  new_sandbox "$1"
  hermit_run FAKE_ARCH=aarch64 -- check
  assert_rc 1
  assert_out_has "x86-64"
}

t_check_refuses_paranoid_3_for_a_user_and_says_it_changes_nothing() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/perf_event_paranoid"
  hermit_run -- check
  assert_rc 1
  assert_out_has "perf_event_paranoid is 3"
  assert_out_has "never changes it"
  assert_no_tripwire
  assert_log_lacks "sysctl"
}

t_check_accepts_paranoid_3_for_root() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/perf_event_paranoid"
  hermit_run FAKE_UID=0 -- check
  assert_rc 0
}

t_check_judges_ptrace_scope() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/yama/ptrace_scope"
  hermit_run FAKE_UID=0 -- check
  assert_rc 1
  assert_out_has "ptrace_scope is 3"
  echo 2 >"$SB/proc/sys/kernel/yama/ptrace_scope"
  hermit_run -- check
  assert_rc 1
  assert_out_has "only root may trace"
  hermit_run FAKE_UID=0 -- check
  assert_rc 0
  rm -f "$SB/proc/sys/kernel/yama/ptrace_scope"
  hermit_run -- check
  assert_rc 0
  assert_out_has "no Yama"
}

t_check_refuses_a_machine_without_a_pmu() {
  new_sandbox "$1"
  rm -rf "$SB/sys/bus/event_source/devices/cpu"
  hermit_run -- check
  assert_rc 1
  assert_out_has "PMU"
  assert_out_lacks "virtual machine"
  echo "flags : fpu hypervisor" >"$SB/proc/cpuinfo"
  hermit_run -- check
  assert_rc 1
  assert_out_has "virtual machine"
}

t_check_accepts_a_hybrid_cpu_pmu_name() {
  new_sandbox "$1"
  rm -rf "$SB/sys/bus/event_source/devices/cpu"
  mkdir -p "$SB/sys/bus/event_source/devices/cpu_core"
  hermit_run -- check
  assert_rc 0
}

t_check_warns_about_an_old_kernel_without_failing() {
  new_sandbox "$1"
  hermit_run FAKE_KERNEL=5.15.0-1-azure -- check
  assert_rc 0
  assert_out_has "older than 6.9"
}

t_check_lists_every_blocker() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/perf_event_paranoid"
  echo 3 >"$SB/proc/sys/kernel/yama/ptrace_scope"
  rm -rf "$SB/sys/bus/event_source/devices/cpu"
  hermit_run -- check
  assert_rc 1
  assert_count "$SB/out" "  - " 3
}

t_check_notes_a_container_and_the_root_caveat() {
  new_sandbox "$1"
  : >"$SB/root/.dockerenv"
  hermit_run -- check
  assert_rc 0
  assert_out_has "running in a container"
  assert_out_has "ran Hermit as root"
}

# ---- install-hermit.sh: the install -----------------------------------------------------------------------------
t_install_builds_verifies_and_installs() {
  new_sandbox "$1"
  local prefix="$SB/prefix" expected
  hermit_run -- install --prefix "$prefix"
  assert_rc 0
  assert_exists "$prefix/bin/hermit"
  expected="$(sha256_of "$SB/fake-hermit")"
  assert_out_has "sha256:    $expected"
  assert_out_has "export HAIRSPRING_HERMIT=$prefix/bin/hermit"
  assert_out_has "patch sha256 verified"
  assert_out_has "commit verified: $PINNED_COMMIT"
  assert_log_has "toolchain=$PINNED_TOOLCHAIN"
  assert_log_has "cargo build --release --locked"
  assert_log_has "-p hermit --bin hermit"
  assert_no_tripwire
  assert_exists "$prefix/hermit.provenance"
  grep -qF "commit=$PINNED_COMMIT" "$prefix/hermit.provenance" || fail_now "provenance lacks the commit"
  grep -qF "patch_sha256=$(sha256_of "$REAL_PATCH")" "$prefix/hermit.provenance" || fail_now "provenance lacks the patch hash"
  grep -qF "binary_sha256=$expected" "$prefix/hermit.provenance" || fail_now "provenance lacks the binary hash"
  grep -qF "version=hermit 0.4.1" "$prefix/hermit.provenance" || fail_now "provenance lacks the version"
  [ -z "$(ls -A "$SB/tmp")" ] || fail_now "the temporary build directory was not removed" "$(ls "$SB/tmp")"
}

t_install_rejects_a_tampered_patch_before_touching_anything() {
  new_sandbox "$1"
  cp "$REAL_PATCH" "$SB/tampered.diff"
  printf '\n' >>"$SB/tampered.diff"
  PATCH_UNDER_TEST="$SB/tampered.diff" hermit_run -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "does not match its pin"
  assert_log_lacks "git "
  assert_log_lacks "cargo "
  assert_absent "$SB/prefix"
}

t_install_rejects_a_missing_patch() {
  new_sandbox "$1"
  PATCH_UNDER_TEST="$SB/nope.diff" hermit_run -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "replay patch is not at"
  assert_absent "$SB/prefix"
}

t_install_rejects_a_checkout_at_the_wrong_commit() {
  new_sandbox "$1"
  hermit_run FAKE_GIT_HEAD=0000000000000000000000000000000000000000 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "not the pinned commit"
  assert_log_lacks "cargo build"
  assert_absent "$SB/prefix"
}

t_install_stops_when_the_patch_does_not_apply() {
  new_sandbox "$1"
  hermit_run FAKE_APPLY_CHECK_RC=1 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "does not apply"
  assert_log_lacks "cargo build"
  assert_absent "$SB/prefix"
}

t_install_names_the_toolchain_command_when_it_is_missing() {
  new_sandbox "$1"
  hermit_run FAKE_NO_TOOLCHAIN=1 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "rustup toolchain install $PINNED_TOOLCHAIN --profile minimal"
  assert_log_lacks "git clone"
  assert_log_lacks "toolchain install"
  assert_absent "$SB/prefix"
}

t_install_toolchain_flag_installs_it_through_rustup() {
  new_sandbox "$1"
  hermit_run FAKE_NO_TOOLCHAIN=1 -- install --prefix "$SB/prefix" --install-toolchain
  assert_rc 0
  assert_log_has "rustup toolchain install $PINNED_TOOLCHAIN --profile minimal"
  assert_exists "$SB/prefix/bin/hermit"
  assert_no_tripwire
}

t_install_names_missing_development_libraries() {
  new_sandbox "$1"
  hermit_run "FAKE_MISSING_PC=libunwind liblzma" -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "libunwind-dev"
  assert_err_has "liblzma-dev"
  assert_log_lacks "git clone"
}

t_install_names_a_missing_git() {
  new_sandbox "$1"
  rm -f "$BIN/git"
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "git"
  assert_absent "$SB/prefix"
}

t_install_refuses_other_platforms() {
  new_sandbox "$1"
  hermit_run FAKE_ARCH=aarch64 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "Linux x86-64"
  assert_log_lacks "git clone"
}

t_install_rejects_a_binary_without_the_log_flag() {
  new_sandbox "$1"
  hermit_run FAKE_HERMIT_NO_LOG=1 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "--log"
  assert_absent "$SB/prefix/bin/hermit"
  assert_err_has "build directory kept"
}

t_install_rejects_a_binary_that_lacks_a_flag_the_backend_passes() {
  new_sandbox "$1"
  hermit_run FAKE_HERMIT_MISSING_FLAG=--sched-seed -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "--sched-seed"
  assert_absent "$SB/prefix/bin/hermit"
}

t_install_rejects_a_binary_that_cannot_print_its_version() {
  new_sandbox "$1"
  hermit_run FAKE_HERMIT_VERSION_FAIL=segfault -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "--version"
  assert_absent "$SB/prefix/bin/hermit"
}

t_install_explains_a_close_range_failure_and_the_skip_flag() {
  new_sandbox "$1"
  hermit_run "FAKE_HERMIT_VERSION_FAIL=Inherited descriptor cleanup failed: close_range: Function not implemented" -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "close_range"
  assert_err_has "--skip-run-verify"
}

t_skip_run_verify_installs_but_says_unverified() {
  new_sandbox "$1"
  hermit_run FAKE_HERMIT_VERSION_FAIL=segfault -- install --prefix "$SB/prefix" --skip-run-verify
  assert_rc 0
  assert_exists "$SB/prefix/bin/hermit"
  assert_err_has "UNVERIFIED"
  grep -qF "version=unverified" "$SB/prefix/hermit.provenance" || fail_now "provenance should say unverified"
}

t_skip_run_verify_repeats_unverified_in_the_closing_summary() {
  new_sandbox "$1"
  hermit_run -- install --prefix "$SB/prefix" --skip-run-verify
  assert_rc 0
  grep -E '^installed: .*UNVERIFIED' "$SB/out" >/dev/null || fail_now "the installed: line lacks UNVERIFIED" "$(cat "$SB/out")"
}

t_a_verified_install_does_not_claim_unverified_in_its_summary() {
  new_sandbox "$1"
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 0
  grep -E '^installed: ' "$SB/out" | grep -qv UNVERIFIED || fail_now "a verified install said UNVERIFIED" "$(cat "$SB/out")"
}

t_patch_changed_after_the_first_check_is_refused_before_it_is_applied() {
  new_sandbox "$1"
  cp "$REAL_PATCH" "$SB/swap.diff"
  PATCH_UNDER_TEST="$SB/swap.diff" hermit_run FAKE_TAMPER_PATCH=1 -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "changed after it was verified"
  assert_log_lacks " apply "
  assert_absent "$SB/prefix/bin/hermit"
}

t_check_rejects_install_only_options() {
  new_sandbox "$1"
  hermit_run -- check --prefix "$SB/prefix"
  assert_rc 2
  assert_err_has "install option"
  hermit_run -- check --skip-run-verify
  assert_rc 2
}

t_install_refuses_a_foreign_non_empty_prefix() {
  new_sandbox "$1"
  mkdir -p "$SB/prefix"
  echo precious >"$SB/prefix/notes.txt"
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "not created by this script"
  assert_log_lacks "git clone"
  [ "$(cat "$SB/prefix/notes.txt")" = "precious" ] || fail_now "the foreign file was changed"
}

t_install_refuses_the_root_prefix() {
  new_sandbox "$1"
  hermit_run -- install --prefix /
  assert_rc 1
  assert_err_has "refusing"
  assert_log_lacks "git clone"
}

t_install_can_replace_its_own_earlier_install() {
  new_sandbox "$1"
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 0
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 0
}

t_install_dry_run_builds_and_writes_nothing() {
  new_sandbox "$1"
  hermit_run -- install --prefix "$SB/prefix" --dry-run
  assert_rc 0
  assert_out_has "dry run"
  assert_out_has "$PINNED_COMMIT"
  assert_log_lacks "git clone"
  assert_log_lacks "cargo build"
  assert_absent "$SB/prefix"
}

t_install_dry_run_still_verifies_the_pins() {
  new_sandbox "$1"
  cp "$REAL_PATCH" "$SB/tampered.diff"
  printf 'x' >>"$SB/tampered.diff"
  PATCH_UNDER_TEST="$SB/tampered.diff" hermit_run -- install --prefix "$SB/prefix" --dry-run
  assert_rc 1
}

t_install_keeps_a_user_work_dir_and_refuses_a_non_empty_one() {
  new_sandbox "$1"
  hermit_run -- install --prefix "$SB/prefix" --work-dir "$SB/work"
  assert_rc 0
  assert_exists "$SB/work/hermit/target/release/hermit"
  hermit_run -- install --prefix "$SB/prefix2" --work-dir "$SB/work"
  assert_rc 1
  assert_err_has "is not empty"
}

t_install_reports_a_host_that_cannot_run_hermit_but_still_succeeds() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/perf_event_paranoid"
  hermit_run -- install --prefix "$SB/prefix"
  assert_rc 0
  assert_exists "$SB/prefix/bin/hermit"
  assert_out_has "CANNOT RUN"
  assert_out_has "this machine cannot run it"
}

t_install_require_host_fails_when_the_host_cannot_run_hermit() {
  new_sandbox "$1"
  echo 3 >"$SB/proc/sys/kernel/perf_event_paranoid"
  hermit_run -- install --prefix "$SB/prefix" --require-host
  assert_rc 1
}

# ---- install.sh -------------------------------------------------------------------------------------------------
t_cli_help_lists_the_options() {
  new_sandbox "$1"
  installer_run -- --help
  assert_rc 0
  assert_out_has "--version TAG"
  assert_out_has "SHA256SUMS"
}

t_cli_rejects_bad_arguments() {
  new_sandbox "$1"
  installer_run -- --frobnicate
  assert_rc 2
  installer_run -- --version latest
  assert_rc 2
  installer_run -- --version 'v1/../x'
  assert_rc 2
  installer_run -- --version v1.0.0 --repo 'not a repo'
  assert_rc 2
  installer_run -- --version v1.0.0 --repo 'a/b/c'
  assert_rc 2
  installer_run -- --version v1.0.0 --repo 'a b/c'
  assert_rc 2
  installer_run -- --version v1.0.0 --repo 'owner/repo;rm'
  assert_rc 2
  installer_run -- --sha256 "$(printf 'a%.0s' $(seq 1 64))"
  assert_rc 2
  installer_run -- --version v1.0.0 --sha256 deadbeef
  assert_rc 2
  installer_run -- --version v1.0.0 --from-source
  assert_rc 2
  installer_run -- --version v1.0.0 --with-ui
  assert_rc 2
}

t_cli_download_verifies_installs_and_runs_doctor() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 0
  assert_exists "$SB/prefix/bin/hairspring"
  [ -x "$SB/prefix/bin/hairspring" ] || fail_now "not executable"
  assert_out_has "sha256 verified: $(sha256_of "$SB/rel/hairspring-linux-x86_64")"
  assert_out_has "version: hairspring 9.9.9"
  assert_out_has "lock-lease: deterministic"
  assert_out_has "hairspring doctor --process-backend"
  assert_out_has "cannot run the process backend"
  assert_no_tripwire
}

t_cli_download_only_follows_https_redirects_with_tls_1_2() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  mk_fake curl "exec \"$TMP/real/curl\" \"\$@\""
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 0
  assert_log_has "--proto-redir =https"
  assert_log_has "--tlsv1.2"
}

t_cli_download_picks_the_macos_asset_on_apple_silicon() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-macos-arm64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_OS=Darwin FAKE_ARCH=arm64 -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 0
  assert_exists "$SB/prefix/bin/hairspring"
}

t_cli_download_refuses_platforms_without_an_asset() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_ARCH=aarch64 -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "--from-source"
  assert_absent "$SB/prefix"
}

t_cli_download_rejects_a_tampered_binary() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  printf '\n# tampered\n' >>"$SB/rel/hairspring-linux-x86_64"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "sha256 mismatch"
  assert_absent "$SB/prefix/bin/hairspring"
}

t_cli_download_rejects_an_asset_missing_from_the_sums() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  : >"$SB/rel/SHA256SUMS"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "lists 0 entries"
  assert_absent "$SB/prefix/bin/hairspring"
}

t_cli_download_rejects_two_conflicting_sums_entries() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  printf '%s  hairspring-linux-x86_64\n' "$(printf 'b%.0s' $(seq 1 64))" >>"$SB/rel/SHA256SUMS"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "lists 2 entries"
}

t_cli_download_rejects_a_malformed_hash_in_the_sums() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  printf 'nothex  hairspring-linux-x86_64\n' >"$SB/rel/SHA256SUMS"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "malformed hash"
}

t_cli_download_fails_clearly_when_a_file_cannot_be_fetched() {
  new_sandbox "$1"
  mkdir -p "$SB/rel"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "download failed"
  assert_absent "$SB/prefix"
}

t_cli_download_enforces_an_out_of_band_sha256() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix" --sha256 "$(printf 'c%.0s' $(seq 1 64))"
  assert_rc 1
  assert_err_has "not the --sha256 you gave"
  assert_absent "$SB/prefix/bin/hairspring"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix" --sha256 "$(sha256_of "$SB/rel/hairspring-linux-x86_64")"
  assert_rc 0
}

t_cli_download_does_not_install_a_binary_that_will_not_run() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_HS_VERSION_FAIL=1 -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "does not run"
  assert_absent "$SB/prefix/bin/hairspring"
}

t_cli_download_does_not_install_a_binary_with_the_wrong_name() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_HS_VERSION=oops -- --version v0.1.0 --prefix "$SB/prefix"
  # "hairspring oops" does not start with a digit
  assert_rc 1
  assert_absent "$SB/prefix/bin/hairspring"
}

t_cli_install_fails_when_doctor_fails() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_DOCTOR_RC=1 -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "doctor' failed"
}

t_cli_process_backend_is_information_unless_required() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 0
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix" --require-process-backend
  assert_rc 1
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_BACKEND_OK=1 -- --version v0.1.0 --prefix "$SB/prefix" --require-process-backend
  assert_rc 0
  assert_out_has "prerequisites are met"
}

t_cli_an_older_build_without_the_backend_check_is_a_warning() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_BACKEND_RC=2 -- --version v0.1.0 --prefix "$SB/prefix"
  assert_rc 0
  assert_err_has "may predate"
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" FAKE_BACKEND_RC=2 -- --version v0.1.0 --prefix "$SB/prefix" --require-process-backend
  assert_rc 1
}

t_cli_prefix_may_contain_spaces() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/pre fix"
  assert_rc 0
  assert_exists "$SB/pre fix/bin/hairspring"
}

t_cli_dry_run_downloads_and_writes_nothing() {
  new_sandbox "$1"
  make_release "$SB/rel" hairspring-linux-x86_64
  installer_run "HAIRSPRING_RELEASE_BASE_URL=file://$SB/rel" -- --version v0.1.0 --prefix "$SB/prefix" --dry-run
  assert_rc 0
  assert_out_has "dry run"
  assert_absent "$SB/prefix"
  assert_log_lacks "cargo"
}

t_cli_source_build_uses_locked_and_installs() {
  new_sandbox "$1"
  installer_run -- --prefix "$SB/prefix"
  assert_rc 0
  assert_log_has "cargo build --release --locked -p hairspring-cli"
  assert_exists "$SB/prefix/bin/hairspring"
  assert_out_has "version: hairspring 9.9.9"
  assert_no_tripwire
}

t_cli_source_build_names_a_missing_cargo() {
  new_sandbox "$1"
  rm -f "$BIN/cargo"
  installer_run -- --prefix "$SB/prefix"
  assert_rc 1
  assert_err_has "cargo is required"
  assert_absent "$SB/prefix"
}

t_cli_source_build_fails_when_the_build_fails() {
  new_sandbox "$1"
  mk_fake cargo 'echo "error: could not compile" >&2; exit 101'
  installer_run -- --prefix "$SB/prefix"
  assert_rc 101
  assert_absent "$SB/prefix/bin/hairspring"
}

# ---- test-install-docker.sh ---------------------------------------------------------------------------------------
t_docker_script_dry_run_prints_the_plan_and_starts_nothing() {
  new_sandbox "$1"
  mk_fake docker 'exit 0'
  runx -- bash "$SCRIPTS_DIR/test-install-docker.sh" --dry-run
  assert_rc 0
  assert_out_has "ubuntu:24.04 ubuntu:22.04"
  assert_out_has "dry run: no container was started"
  assert_log_lacks "docker run"
}

t_docker_script_rejects_unknown_options() {
  new_sandbox "$1"
  runx -- bash "$SCRIPTS_DIR/test-install-docker.sh" --frobnicate
  assert_rc 2
}

# ---- runner -----------------------------------------------------------------------------------------------------
run_test() {
  CURRENT="$1"
  TEST_FAILED=0
  "$1" "$1"
  if [ "$TEST_FAILED" = 0 ]; then
    PASS=$((PASS + 1))
    printf 'ok   %s\n' "$1"
  fi
}

run_all() {
  link_real_tools
  local fn
  for fn in $(declare -F | awk '{print $3}' | grep '^t_'); do
    run_test "$fn"
  done
  printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
  [ "$FAIL" = 0 ]
}

# ---- mutation run -----------------------------------------------------------------------------------------------
# Each mutant is a copy of one script with one guard weakened. The suite must fail on every one of them; a mutant the
# suite accepts means a check nobody tests.
SURVIVORS=0
mutate() {
  # $1 name, $2 script, $3 text to replace (literal), $4 replacement (literal)
  local name="$1" file="$2" old="$3" new="$4" dir content mutated
  dir="$TMP/mut/$name"
  mkdir -p "$dir/scripts" "$dir/crates/hairspring-cli"
  : >"$dir/Cargo.toml"
  cp "$HERE"/install-hermit.sh "$HERE"/install.sh "$HERE"/test-install-*.sh "$dir/scripts/"
  content="$(cat "$HERE/$file")"
  mutated="${content/"$old"/"$new"}"
  if [ "$mutated" = "$content" ]; then
    printf 'MUTANT %s: the text to replace was not found in %s\n' "$name" "$file"
    SURVIVORS=$((SURVIVORS + 1))
    return
  fi
  printf '%s\n' "$mutated" >"$dir/scripts/$file"
  if HS_SCRIPTS_DIR="$dir/scripts" bash "$HERE/test-scripts.sh" >"$dir/log" 2>&1; then
    printf 'SURVIVED %s (the suite still passes)\n' "$name"
    SURVIVORS=$((SURVIVORS + 1))
  else
    printf 'killed   %s\n' "$name"
  fi
}

# Control: an unmutated copy built the same way as a mutant must pass the whole suite. If it does not, every mutant
# would count as killed for the wrong reason, so the run stops here.
run_control() {
  local dir="$TMP/mut/control"
  mkdir -p "$dir/scripts" "$dir/crates/hairspring-cli"
  : >"$dir/Cargo.toml"
  cp "$HERE"/install-hermit.sh "$HERE"/install.sh "$HERE"/test-install-*.sh "$dir/scripts/"
  if ! HS_SCRIPTS_DIR="$dir/scripts" bash "$HERE/test-scripts.sh" >"$dir/log" 2>&1; then
    printf 'CONTROL FAILED: an unmutated copy does not pass the suite, so no mutant result can be trusted.\n' >&2
    tail -n 30 "$dir/log" >&2
    return 1
  fi
  printf 'control  an unmutated copy passes the whole suite\n'
}

run_mutations() {
  run_control || return 1
  mutate hermit-patch-pin-ignored install-hermit.sh 'if [ "$actual" != "$HERMIT_PATCH_SHA256" ]; then' 'if false; then'
  mutate hermit-commit-pin-ignored install-hermit.sh 'if [ "$head" != "$HERMIT_BASE_COMMIT" ]; then' 'if false; then'
  mutate hermit-log-flag-unchecked install-hermit.sh '*--log*) ;;' '*) ;;'
  mutate hermit-run-flags-unchecked install-hermit.sh 'for flag in $REQUIRED_RUN_FLAGS; do' 'for flag in ; do'
  mutate hermit-paranoid-ignored install-hermit.sh '[ "$paranoid" -ge "$PARANOID_DENIES_ALL" ]' 'false'
  mutate hermit-ptrace-disabled-ignored install-hermit.sh '[ "$scope" -ge "$PTRACE_DISABLED" ]' 'false'
  mutate hermit-ptrace-admin-ignored install-hermit.sh '[ "$scope" -eq "$PTRACE_ADMIN_ONLY" ] && ! is_root' 'false'
  mutate hermit-pmu-ignored install-hermit.sh 'if [ -e "$devices/cpu" ] || [ -e "$devices/cpu_core" ]; then' 'if true; then'
  mutate hermit-check-always-succeeds install-hermit.sh '  printf '"'"'%s'"'"' "$BLOCKERS"
  return 1' '  printf '"'"'%s'"'"' "$BLOCKERS"
  return 0'
  mutate hermit-uses-sudo install-hermit.sh 'rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal' 'sudo rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal'
  mutate hermit-changes-sysctl install-hermit.sh '    row perf FAIL "perf_event_paranoid = $paranoid"' '    sysctl -w kernel.perf_event_paranoid=1; row perf FAIL "perf_event_paranoid = $paranoid"'
  mutate hermit-foreign-prefix-allowed install-hermit.sh '  [ -z "$(ls -A "$PREFIX")" ] && return 0' '  return 0'
  mutate hermit-unlocked-build install-hermit.sh 'cargo build --release --locked' 'cargo build --release'
  mutate hermit-unpinned-toolchain install-hermit.sh 'RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN" cargo' 'cargo'
  mutate hermit-installs-before-verifying install-hermit.sh '  verify_hermit_binary "$src/target/release/hermit"
  install_binary' '  install_binary'
  mutate hermit-platform-unchecked install-hermit.sh '  if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
    add_missing' '  if false; then
    add_missing'
  mutate hermit-temp-dir-never-removed install-hermit.sh '      rm -rf "$TEMP_WORK"' '      true'
  mutate hermit-require-host-ignored install-hermit.sh '[ "$REQUIRE_HOST" != "yes" ] || exit 1' 'true'
  mutate hermit-patch-not-rechecked-before-apply install-hermit.sh 'if [ "$actual" != "$HERMIT_PATCH_SHA256" ]; then
    die "the replay patch changed after' 'if false; then
    die "the replay patch changed after'
  mutate hermit-summary-hides-unverified install-hermit.sh '    say "installed: $PREFIX/bin/hermit (UNVERIFIED' '    say "installed: $PREFIX/bin/hermit (checked'
  mutate hermit-check-accepts-install-options install-hermit.sh '  if [ "$COMMAND" = "check" ]; then' '  if false; then'
  mutate cli-redirects-not-restricted install.sh ' --proto-redir =https' ''
  mutate cli-tls-version-unpinned install.sh ' --tlsv1.2' ''
  mutate cli-sha256-compare-ignored install.sh 'if [ "$actual" != "$expected" ]; then' 'if false; then'
  mutate cli-pinned-sha256-ignored install.sh 'if [ -n "$PINNED_SHA256" ] && [ "$actual" != "$PINNED_SHA256" ]; then' 'if false; then'
  mutate cli-sums-count-unchecked install.sh '[ "$count" -eq 1 ] ||' 'true ||'
  mutate cli-sums-hash-shape-unchecked install.sh 'is_hex_sha256 "$found" ||' 'true ||'
  mutate cli-unlocked-build install.sh '&& cargo build --release --locked -p hairspring-cli)' '&& cargo build --release -p hairspring-cli)'
  mutate cli-version-shape-unchecked install.sh 'hairspring\ [0-9]*) ;;' '*) ;;'
  mutate cli-installs-before-verifying install.sh '  verify_staged_binary "$STAGED"
  install_staged' '  install_staged'
  mutate cli-require-backend-ignored install.sh '[ "$REQUIRE_PROCESS_BACKEND" != "yes" ] || exit 1' 'true'
  mutate cli-doctor-failure-ignored install.sh '"$bin" doctor) || die "'"'"'hairspring doctor'"'"' failed"' '"$bin" doctor) || true'
  mutate cli-tag-unchecked install.sh '      v[0-9]*) ;;
      *) usage_error "--version must be a release tag such as v0.1.0" ;;' '      *) ;;'
  mutate cli-tag-characters-unchecked install.sh '*[!0-9A-Za-z._-]*) usage_error "--version contains characters a tag cannot have" ;;' '*[!0-9A-Za-z._-]*) true ;;'
  mutate cli-repo-unchecked install.sh '*/*/* | */ | /* | *[!0-9A-Za-z._/-]* | '"''"') usage_error' '*/*/* | */ | /* | '"''"') usage_error'
  mutate cli-uses-sudo install.sh '  mv -f "$tmp" "$target"' '  sudo mv -f "$tmp" "$target"'
  mutate cli-platform-unchecked install.sh '    Linux/x86_64) printf '"'"'%s'"'"' "$LINUX_ASSET" ;;' '    Linux/*) printf '"'"'%s'"'"' "$LINUX_ASSET" ;;'
  mutate cli-dry-run-downloads install.sh '  if [ "$DRY_RUN" = "yes" ]; then' '  if false; then'
  printf '\n%s survivors\n' "$SURVIVORS"
  [ "$SURVIVORS" = 0 ]
}

main() {
  case "${1:-}" in
    '')
      run_all
      ;;
    --mutations)
      local rc=0
      run_all || rc=1
      run_mutations || rc=1
      exit "$rc"
      ;;
    *)
      printf 'usage: test-scripts.sh [--mutations]\n' >&2
      exit 2
      ;;
  esac
}

main "$@"
