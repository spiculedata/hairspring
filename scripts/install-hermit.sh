#!/usr/bin/env bash
# Build the pinned Hermit from source and install it for the Hairspring `process` backend, or check the machine.
#
#   scripts/install-hermit.sh check                       read-only: can this machine run Hermit at all?
#   scripts/install-hermit.sh [install] [options]         build Hermit and install it below a prefix
#
# What it does, in order: verifies the replay patch by sha256, checks the build prerequisites, clones the pinned
# fork, checks out the pinned commit and verifies it, applies the patch, builds with the pinned Rust toolchain,
# verifies the binary (version, `--log`, the flags the backend passes), installs it below the prefix and prints its
# sha256.
#
# What it never does: use sudo, run sysctl, change kernel settings or system packages, or touch anything outside the
# prefix and its own scratch directory. The one opt-in exception is `--install-toolchain`, which runs
# `rustup toolchain install` (a per-user install under ~/.rustup). When a kernel setting is in the way, `check` names
# it and leaves the decision to you.
#
# Test hooks (used by scripts/test-scripts.sh; not for normal use): HAIRSPRING_PROC_ROOT, HAIRSPRING_SYS_ROOT and
# HAIRSPRING_ROOT_DIR replace /proc, /sys and / for the read-only checks; HAIRSPRING_HERMIT_REPO replaces the clone URL;
# HAIRSPRING_HERMIT_PATCH replaces the patch path (its sha256 is still checked); HAIRSPRING_HERMIT_PIN replaces the
# pin file (scripts/hermit.pin).
set -euo pipefail

# ---- pins -------------------------------------------------------------------------------------------------------
# The fork, commit, patch hash, patch path and Rust toolchain are read from scripts/hermit.pin (the one canonical copy;
# scripts/test-scripts.sh checks that the experiment READMEs and the quickstart agree with it). The file is parsed, never
# sourced, and the script refuses to go on if it is missing, has a missing, repeated or unknown key, or has a value of
# the wrong shape.
readonly PROVENANCE_FILE="hermit.provenance"
# Flags the Hairspring backend passes to `hermit run` (crates/hairspring-hermit/src/exec.rs).
readonly REQUIRED_RUN_FLAGS="--epoch --sched-seed --summary-json --chaos --sched-heuristic --max-timeslice --skid-margin"
readonly FAST_KERNEL_MAJOR=6
readonly FAST_KERNEL_MINOR=9
readonly PARANOID_DENIES_ALL=3
readonly PTRACE_ADMIN_ONLY=2
readonly PTRACE_DISABLED=3
readonly DEFAULT_JOBS_CAP=4
readonly MIN_FREE_KB=$((8 * 1024 * 1024))

PROC_ROOT="${HAIRSPRING_PROC_ROOT:-/proc}"
SYS_ROOT="${HAIRSPRING_SYS_ROOT:-/sys}"
ROOT_DIR="${HAIRSPRING_ROOT_DIR:-}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIN_FILE="${HAIRSPRING_HERMIT_PIN:-$SCRIPT_DIR/hermit.pin}"

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}
usage_error() {
  printf 'error: %s\n(see --help)\n' "$*" >&2
  exit 2
}

usage() {
  load_pins
  cat <<EOF
Usage:
  install-hermit.sh check
  install-hermit.sh [install] [--prefix DIR] [--install-toolchain] [--jobs N] [--work-dir DIR]
                    [--keep-work] [--require-host] [--dry-run]

check              Read-only. Reports whether this machine meets Hermit's runtime prerequisites (Linux x86-64, kernel,
                   perf_event_paranoid, ptrace_scope, a hardware PMU). Exit 0 when it does, 1 when it cannot.
install            Build the pinned Hermit and install it (default command).

Options (install):
  --prefix DIR         Install below DIR (bin/hermit and $PROVENANCE_FILE). Default: \$HOME/.local/share/hairspring/hermit
  --install-toolchain  Run 'rustup toolchain install $RUST_TOOLCHAIN --profile minimal' if it is missing. Without this
                       flag a missing toolchain is an error that prints the command.
  --jobs N             Parallel build jobs. Default: the CPU count, at most $DEFAULT_JOBS_CAP.
  --work-dir DIR       Build in DIR (must be empty or absent). It is never deleted. Default: a fresh temporary
                       directory, deleted on success and kept on failure.
  --keep-work          Keep the temporary directory after a successful build.
  --skip-run-verify    Do not run the built binary (for build-only machines such as emulated containers). The install
                       is then reported as UNVERIFIED and no version is recorded.
  --require-host       Exit 1 if the host check at the end finds this machine cannot run Hermit. By default that is
                       reported loudly but the install still succeeds (a build machine need not be a test machine).
  --dry-run            Verify the pins and the build prerequisites and print the plan; build nothing, write nothing.

Pins: fork $HERMIT_REPO_DEFAULT, commit $HERMIT_BASE_COMMIT,
      patch sha256 $HERMIT_PATCH_SHA256, Rust toolchain $RUST_TOOLCHAIN.
Exit codes: 0 success, 1 a prerequisite or verification failed, 2 usage error.
EOF
}

# ---- the pin file -----------------------------------------------------------------------------------------------
HERMIT_REPO_DEFAULT=""
HERMIT_BASE_COMMIT=""
HERMIT_PATCH_SHA256=""
HERMIT_PATCH_REL=""
RUST_TOOLCHAIN=""
HERMIT_REPO=""
HERMIT_PATCH=""
PINS_LOADED="no"

pin_shape_ok() {
  # $1 key, $2 value
  case "$1" in
    HERMIT_REPO) [[ "$2" =~ ^https://[A-Za-z0-9._/-]+$ ]] ;;
    HERMIT_BASE_COMMIT) [[ "$2" =~ ^[0-9a-f]{40}$ ]] ;;
    HERMIT_PATCH_SHA256) [[ "$2" =~ ^[0-9a-f]{64}$ ]] ;;
    HERMIT_PATCH_REL) [[ "$2" =~ ^[A-Za-z0-9._/-]+$ ]] && [[ "$2" != /* ]] && [[ "$2" != *..* ]] ;;
    RUST_TOOLCHAIN) [[ "$2" =~ ^[A-Za-z0-9._-]+$ ]] ;;
    *) return 1 ;;
  esac
}

set_pin() {
  case "$1" in
    HERMIT_REPO) HERMIT_REPO_DEFAULT="$2" ;;
    HERMIT_BASE_COMMIT) HERMIT_BASE_COMMIT="$2" ;;
    HERMIT_PATCH_SHA256) HERMIT_PATCH_SHA256="$2" ;;
    HERMIT_PATCH_REL) HERMIT_PATCH_REL="$2" ;;
    RUST_TOOLCHAIN) RUST_TOOLCHAIN="$2" ;;
  esac
}

load_pins() {
  [ "$PINS_LOADED" = "no" ] || return 0
  [ -f "$PIN_FILE" ] && [ -r "$PIN_FILE" ] || die "the pin file $PIN_FILE is missing or unreadable; refusing to continue without the pins"
  local line key value seen=" "
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    case "$line" in
      *=*) ;;
      *) die "pin file $PIN_FILE: '$line' is not a KEY=VALUE line" ;;
    esac
    key="${line%%=*}"
    value="${line#*=}"
    case "$key" in
      HERMIT_REPO | HERMIT_BASE_COMMIT | HERMIT_PATCH_SHA256 | HERMIT_PATCH_REL | RUST_TOOLCHAIN) ;;
      *) die "pin file $PIN_FILE: $key is not a known pin" ;;
    esac
    case "$seen" in *" $key "*) die "pin file $PIN_FILE: $key is given more than once" ;; esac
    pin_shape_ok "$key" "$value" || die "pin file $PIN_FILE: $key has a malformed value '$value'"
    seen="$seen$key "
    set_pin "$key" "$value"
  done <"$PIN_FILE"
  for key in HERMIT_REPO HERMIT_BASE_COMMIT HERMIT_PATCH_SHA256 HERMIT_PATCH_REL RUST_TOOLCHAIN; do
    case "$seen" in *" $key "*) ;; *) die "pin file $PIN_FILE: $key is missing" ;; esac
  done
  HERMIT_REPO="${HAIRSPRING_HERMIT_REPO:-$HERMIT_REPO_DEFAULT}"
  HERMIT_PATCH="${HAIRSPRING_HERMIT_PATCH:-$SCRIPT_DIR/../$HERMIT_PATCH_REL}"
  PINS_LOADED="yes"
}

# ---- small helpers ----------------------------------------------------------------------------------------------
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    die "neither sha256sum nor shasum is available"
  fi
}

read_first() {
  [ -r "$1" ] || return 1
  head -n 1 "$1"
}

is_root() { [ "$(id -u)" = "0" ]; }

row() { printf '  %-14s %-4s  %s\n' "$1" "$2" "$3"; }

# ---- the read-only host check -----------------------------------------------------------------------------------
BLOCKERS=""
add_blocker() { BLOCKERS="${BLOCKERS}  - $1"$'\n'; }

check_kernel_row() {
  local release major rest minor
  release="$(uname -r)"
  major="${release%%.*}"
  rest="${release#*.}"
  minor="${rest%%[!0-9]*}"
  case "$major$minor" in
    '' | *[!0-9]*)
      row kernel note "release '$release' could not be parsed"
      return
      ;;
  esac
  if [ "$major" -lt "$FAST_KERNEL_MAJOR" ] || { [ "$major" -eq "$FAST_KERNEL_MAJOR" ] && [ "$minor" -lt "$FAST_KERNEL_MINOR" ]; }; then
    row kernel note "$release is older than $FAST_KERNEL_MAJOR.$FAST_KERNEL_MINOR: Hermit's ptrace path is much slower here; schedules are unaffected"
  else
    row kernel ok "$release"
  fi
}

check_perf_row() {
  local file="$PROC_ROOT/sys/kernel/perf_event_paranoid" paranoid
  if [ ! -e "$file" ]; then
    row perf FAIL "$file does not exist"
    add_blocker "this kernel has no perf events interface ($file is missing); Hermit needs hardware performance counters."
    return
  fi
  paranoid="$(read_first "$file" || true)"
  case "$paranoid" in
    '' | *[!0-9-]*)
      row perf note "perf_event_paranoid is not readable"
      return
      ;;
  esac
  if is_root; then
    row perf ok "perf_event_paranoid = $paranoid (running as root)"
  elif [ "$paranoid" -ge "$PARANOID_DENIES_ALL" ]; then
    row perf FAIL "perf_event_paranoid = $paranoid"
    add_blocker "kernel.perf_event_paranoid is $paranoid, which denies performance counters to unprivileged users. Run as root, or lower it yourself if your policy allows (for example 'sudo sysctl kernel.perf_event_paranoid=1'; a security trade-off for you to decide). This script never changes it."
  else
    row perf ok "perf_event_paranoid = $paranoid"
  fi
}

check_ptrace_row() {
  local file="$PROC_ROOT/sys/kernel/yama/ptrace_scope" scope
  scope="$(read_first "$file" || true)"
  case "$scope" in
    '') row ptrace ok "no Yama ptrace restriction" ;;
    *[!0-9]*) row ptrace note "ptrace_scope '$scope' is not a number" ;;
    *)
      if [ "$scope" -ge "$PTRACE_DISABLED" ]; then
        row ptrace FAIL "kernel.yama.ptrace_scope = $scope"
        add_blocker "kernel.yama.ptrace_scope is $scope: tracing is disabled until reboot, so Hermit cannot trace a program. This script never changes it."
      elif [ "$scope" -eq "$PTRACE_ADMIN_ONLY" ] && ! is_root; then
        row ptrace FAIL "kernel.yama.ptrace_scope = $scope"
        add_blocker "kernel.yama.ptrace_scope is $scope: only root may trace. Run Hermit as root, or change the setting yourself if your policy allows. This script never changes it."
      else
        row ptrace ok "kernel.yama.ptrace_scope = $scope"
      fi
      ;;
  esac
}

check_pmu_row() {
  local devices="$SYS_ROOT/bus/event_source/devices" virtual=""
  if [ -e "$devices/cpu" ] || [ -e "$devices/cpu_core" ]; then
    row pmu ok "a CPU performance monitoring unit is registered"
    return
  fi
  if grep -qs '^flags.* hypervisor' "$PROC_ROOT/cpuinfo"; then
    virtual=" This looks like a virtual machine that does not pass the PMU through."
  fi
  row pmu FAIL "no hardware PMU registered"
  add_blocker "no CPU performance monitoring unit (PMU) is registered under $devices.$virtual Hermit needs hardware counters: use bare metal or a VM that exposes them."
}

host_report() {
  local os arch
  BLOCKERS=""
  say "Hermit host check (read-only: nothing on this machine is changed)"
  os="$(uname -s)"
  arch="$(uname -m)"
  if [ "$os" != "Linux" ]; then
    row platform FAIL "$os $arch"
    add_blocker "Hermit runs only on Linux; this is $os."
  elif [ "$arch" != "x86_64" ]; then
    row platform FAIL "$os $arch"
    add_blocker "Hermit runs only on x86-64; this machine is $arch."
  else
    row platform ok "$os $arch"
    check_kernel_row
    check_perf_row
    check_ptrace_row
    check_pmu_row
    if ! is_root; then
      row root note "the measurements behind Hairspring's claims ran Hermit as root; running it unprivileged was not tested"
    fi
    if [ -e "$ROOT_DIR/.dockerenv" ] || [ -e "$ROOT_DIR/run/.containerenv" ]; then
      row container note "running in a container: counters and ptrace are often blocked here"
    fi
  fi
  if [ -z "$BLOCKERS" ]; then
    say "result: ok: this machine meets Hermit's runtime prerequisites (nothing has been run under Hermit by this check)"
    say "For the authoritative probe, which opens a counter, run: hairspring doctor --process-backend"
    return 0
  fi
  say "result: CANNOT RUN Hermit on this machine:"
  printf '%s' "$BLOCKERS"
  return 1
}

# ---- build prerequisites ----------------------------------------------------------------------------------------
MISSING=""
add_missing() { MISSING="${MISSING}  - $1"$'\n'; }

have_dev_library() {
  # $1 pkg-config name, $2 header
  if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists "$1" 2>/dev/null; then
    return 0
  fi
  [ -e "$ROOT_DIR/usr/include/$2" ] || [ -e "$ROOT_DIR/usr/local/include/$2" ]
}

toolchain_installed() {
  rustup toolchain list 2>/dev/null | grep -q "^$RUST_TOOLCHAIN"
}

check_build_prereqs() {
  MISSING=""
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
    add_missing "a Linux x86-64 machine: Hermit builds and runs only there (this is $os $arch)"
  fi
  command -v git >/dev/null 2>&1 || add_missing "git (Debian/Ubuntu: apt-get install git)"
  if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1; then
    add_missing "a C compiler (Debian/Ubuntu: apt-get install build-essential)"
  fi
  command -v pkg-config >/dev/null 2>&1 || add_missing "pkg-config (Debian/Ubuntu: apt-get install pkg-config)"
  have_dev_library libunwind libunwind.h || add_missing "libunwind development files (Debian/Ubuntu: apt-get install libunwind-dev)"
  have_dev_library liblzma lzma.h || add_missing "liblzma development files (Debian/Ubuntu: apt-get install liblzma-dev)"
  if ! command -v rustup >/dev/null 2>&1; then
    add_missing "rustup (https://rustup.rs): the pinned toolchain $RUST_TOOLCHAIN is installed through it"
  elif ! command -v cargo >/dev/null 2>&1; then
    add_missing "cargo on PATH (the rustup proxy)"
  elif ! toolchain_installed && [ "$INSTALL_TOOLCHAIN" != "yes" ]; then
    add_missing "the Rust toolchain $RUST_TOOLCHAIN. Install it with: rustup toolchain install $RUST_TOOLCHAIN --profile minimal   (or re-run with --install-toolchain)"
  fi
  if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    add_missing "sha256sum or shasum"
  fi
  [ -z "$MISSING" ]
}

# ---- install ----------------------------------------------------------------------------------------------------
PREFIX="${HOME:-}/.local/share/hairspring/hermit"
INSTALL_TOOLCHAIN="no"
JOBS=""
WORK_DIR=""
KEEP_WORK="no"
SKIP_RUN_VERIFY="no"
REQUIRE_HOST="no"
DRY_RUN="no"
TEMP_WORK=""

verify_patch_pin() {
  [ -f "$HERMIT_PATCH" ] || die "the replay patch is not at $HERMIT_PATCH (run this script from a Hairspring checkout)"
  local actual
  actual="$(sha256_of "$HERMIT_PATCH")"
  if [ "$actual" != "$HERMIT_PATCH_SHA256" ]; then
    die "the replay patch does not match its pin: sha256 is $actual, expected $HERMIT_PATCH_SHA256 ($HERMIT_PATCH). Refusing to build."
  fi
  say "patch sha256 verified: $actual"
}

prefix_is_safe() {
  [ -n "$PREFIX" ] || die "the install prefix is empty"
  [ "$PREFIX" != "/" ] || die "refusing to install into /"
  [ ! -e "$PREFIX" ] && return 0
  [ -d "$PREFIX" ] || die "$PREFIX exists and is not a directory"
  [ -z "$(ls -A "$PREFIX")" ] && return 0
  [ -f "$PREFIX/$PROVENANCE_FILE" ] && return 0
  die "$PREFIX is not empty and was not created by this script (no $PROVENANCE_FILE); refusing to write into it. Choose another --prefix."
}

verify_hermit_binary() {
  local bin="$1" version help run_help flag
  [ -x "$bin" ] || die "$bin is not an executable file"
  if [ "$SKIP_RUN_VERIFY" = "yes" ]; then
    warn "UNVERIFIED: --skip-run-verify was given, so $bin was not run; its version and flags were not checked"
    return 0
  fi
  if ! version="$("$bin" --version 2>&1)"; then
    case "$version" in
      *close_range*)
        die "Hermit cannot start here: this environment does not implement close_range (an emulated or restricted container). Run on a native Linux 5.9+ host, or pass --skip-run-verify to install it without running it. Output: $version"
        ;;
      *) die "'$bin --version' failed: $version" ;;
    esac
  fi
  case "$version" in
    *[Hh]ermit*) ;;
    *) die "'$bin --version' did not name Hermit: $version" ;;
  esac
  help="$("$bin" --help 2>&1)" || die "'$bin --help' failed"
  case "$help" in
    *--log*) ;;
    *) die "'$bin --help' does not list --log, which the backend relies on for diagnostics" ;;
  esac
  case "$help" in
    *run*) ;;
    *) die "'$bin --help' does not list the 'run' subcommand" ;;
  esac
  run_help="$("$bin" run --help 2>&1)" || die "'$bin run --help' failed"
  for flag in $REQUIRED_RUN_FLAGS; do
    case "$run_help" in
      *"$flag"*) ;;
      *) die "'$bin run --help' does not list $flag, which the Hairspring backend passes" ;;
    esac
  done
  say "verified: $bin prints a version, lists --log and the run flags the backend uses"
  say "version: $(printf '%s' "$version" | head -n 1)"
}

cleanup_temp_work() {
  if [ -n "$TEMP_WORK" ] && [ -d "$TEMP_WORK" ]; then
    if [ "$1" = "success" ] && [ "$KEEP_WORK" != "yes" ]; then
      rm -rf "$TEMP_WORK"
    else
      printf 'build directory kept at %s\n' "$TEMP_WORK" >&2
    fi
  fi
}

choose_jobs() {
  [ -z "$JOBS" ] || return 0
  local cpus
  cpus="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1)"
  case "$cpus" in '' | *[!0-9]*) cpus=1 ;; esac
  JOBS="$cpus"
  [ "$JOBS" -le "$DEFAULT_JOBS_CAP" ] || JOBS="$DEFAULT_JOBS_CAP"
}

prepare_work_dir() {
  if [ -n "$WORK_DIR" ]; then
    mkdir -p "$WORK_DIR"
    [ -z "$(ls -A "$WORK_DIR")" ] || die "--work-dir $WORK_DIR is not empty"
    return 0
  fi
  TEMP_WORK="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-hermit-build.XXXXXX")"
  WORK_DIR="$TEMP_WORK"
}

warn_if_low_disk() {
  local free
  free="$(df -Pk "$1" 2>/dev/null | awk 'NR==2 {print $4}')"
  case "$free" in '' | *[!0-9]*) return 0 ;; esac
  if [ "$free" -lt "$MIN_FREE_KB" ]; then
    warn "only $((free / 1024)) MiB free in $1; a Hermit build needs several GiB"
  fi
}

fetch_pinned_source() {
  local src="$1" head
  say "cloning $HERMIT_REPO"
  git clone --quiet --filter=blob:none --no-checkout "$HERMIT_REPO" "$src"
  git -C "$src" checkout --quiet --detach "$HERMIT_BASE_COMMIT"
  head="$(git -C "$src" rev-parse HEAD)"
  if [ "$head" != "$HERMIT_BASE_COMMIT" ]; then
    die "the checkout is at $head, not the pinned commit $HERMIT_BASE_COMMIT"
  fi
  say "commit verified: $head"
}

apply_patch() {
  local src="$1"
  # Hash and apply one private copy, so the file cannot change between the check and the use.
  local copy="$WORK_DIR/replay.patch" actual
  cp "$HERMIT_PATCH" "$copy"
  actual="$(sha256_of "$copy")"
  if [ "$actual" != "$HERMIT_PATCH_SHA256" ]; then
    die "the replay patch changed after it was verified: sha256 is $actual, expected $HERMIT_PATCH_SHA256. Refusing to apply it."
  fi
  git -C "$src" apply --check "$copy" || die "the replay patch does not apply to $HERMIT_BASE_COMMIT"
  git -C "$src" apply "$copy"
  say "replay patch applied"
}

build_hermit() {
  local src="$1"
  if [ "$INSTALL_TOOLCHAIN" = "yes" ] && ! toolchain_installed; then
    say "installing the Rust toolchain $RUST_TOOLCHAIN (rustup, per-user)"
    rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal
  fi
  if [ -f "$src/rust-toolchain.toml" ] && ! grep -q "$RUST_TOOLCHAIN" "$src/rust-toolchain.toml"; then
    warn "the pinned commit's rust-toolchain.toml does not name $RUST_TOOLCHAIN; building with $RUST_TOOLCHAIN anyway"
  fi
  say "building (toolchain $RUST_TOOLCHAIN, $JOBS jobs): this takes a while"
  (cd "$src" && CARGO_TARGET_DIR="$src/target" RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN" cargo build --release --locked -j "$JOBS" -p hermit --bin hermit)
  [ -x "$src/target/release/hermit" ] || die "the build finished but $src/target/release/hermit does not exist"
}

install_binary() {
  local built="$1" installed="$PREFIX/bin/hermit" version staged
  mkdir -p "$PREFIX/bin"
  staged="$PREFIX/bin/.hermit.new.$$"
  cp "$built" "$staged"
  chmod 0755 "$staged"
  mv -f "$staged" "$installed"
  version="unverified (not run)"
  [ "$SKIP_RUN_VERIFY" = "yes" ] || version="$("$installed" --version 2>&1 | head -n 1)"
  {
    printf 'repo=%s\n' "$HERMIT_REPO"
    printf 'commit=%s\n' "$HERMIT_BASE_COMMIT"
    printf 'patch_sha256=%s\n' "$HERMIT_PATCH_SHA256"
    printf 'toolchain=%s\n' "$RUST_TOOLCHAIN"
    printf 'binary_sha256=%s\n' "$(sha256_of "$installed")"
    printf 'version=%s\n' "$version"
  } >"$PREFIX/$PROVENANCE_FILE"
}

print_plan() {
  say "plan:"
  say "  fork        $HERMIT_REPO"
  say "  commit      $HERMIT_BASE_COMMIT"
  say "  patch       $HERMIT_PATCH (sha256 $HERMIT_PATCH_SHA256)"
  say "  toolchain   $RUST_TOOLCHAIN"
  say "  prefix      $PREFIX"
}

run_install() {
  load_pins
  print_plan
  verify_patch_pin
  if ! check_build_prereqs; then
    printf 'error: cannot build Hermit here; missing:\n%s' "$MISSING" >&2
    exit 1
  fi
  prefix_is_safe
  if [ "$DRY_RUN" = "yes" ]; then
    say "dry run: the pins verify and the build prerequisites are met. Nothing was built or written."
    exit 0
  fi
  choose_jobs
  prepare_work_dir
  trap 'cleanup_temp_work failure' EXIT
  warn_if_low_disk "$WORK_DIR"
  local src="$WORK_DIR/hermit"
  fetch_pinned_source "$src"
  apply_patch "$src"
  build_hermit "$src"
  verify_hermit_binary "$src/target/release/hermit"
  install_binary "$src/target/release/hermit"
  verify_hermit_binary "$PREFIX/bin/hermit"
  trap - EXIT
  cleanup_temp_work success
  if [ "$SKIP_RUN_VERIFY" = "yes" ]; then
    say "installed: $PREFIX/bin/hermit (UNVERIFIED: --skip-run-verify was given, so it was never run)"
  else
    say "installed: $PREFIX/bin/hermit"
  fi
  say "sha256:    $(sha256_of "$PREFIX/bin/hermit")"
  say "A rebuild is not expected to give the same sha256; the provenance is recorded in $PREFIX/$PROVENANCE_FILE."
  say "Use it with: export HAIRSPRING_HERMIT=$PREFIX/bin/hermit"
  if ! host_report; then
    say "note: Hermit is installed, but this machine cannot run it (see above)."
    [ "$REQUIRE_HOST" != "yes" ] || exit 1
  fi
}

# ---- main -------------------------------------------------------------------------------------------------------
COMMAND="install"
case "${1:-}" in
  check | install)
    COMMAND="$1"
    shift
    ;;
esac

while [ "$#" -gt 0 ]; do
  if [ "$COMMAND" = "check" ]; then
    case "$1" in -h | --help) ;; *) usage_error "$1 is an install option; 'check' takes no options" ;; esac
  fi
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --prefix)
      [ "$#" -ge 2 ] || usage_error "--prefix needs a directory"
      PREFIX="$2"
      shift 2
      ;;
    --jobs)
      [ "$#" -ge 2 ] || usage_error "--jobs needs a number"
      case "$2" in '' | *[!0-9]* | 0) usage_error "--jobs needs a positive number" ;; esac
      JOBS="$2"
      shift 2
      ;;
    --work-dir)
      [ "$#" -ge 2 ] || usage_error "--work-dir needs a directory"
      WORK_DIR="$2"
      shift 2
      ;;
    --install-toolchain)
      INSTALL_TOOLCHAIN="yes"
      shift
      ;;
    --keep-work)
      KEEP_WORK="yes"
      shift
      ;;
    --skip-run-verify)
      SKIP_RUN_VERIFY="yes"
      shift
      ;;
    --require-host)
      REQUIRE_HOST="yes"
      shift
      ;;
    --dry-run)
      DRY_RUN="yes"
      shift
      ;;
    *) usage_error "unknown option: $1" ;;
  esac
done

case "$COMMAND" in
  check) host_report ;;
  install) run_install ;;
esac
