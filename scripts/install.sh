#!/usr/bin/env bash
# Install the `hairspring` command line, from a release asset (checked against its sha256 first) or from source.
#
#   scripts/install.sh                         build this checkout with 'cargo build --release --locked' and install it
#   scripts/install.sh --version v0.1.0        download that GitHub release's binary, verify it, install it
#
# After installing it checks `hairspring --version`, runs `hairspring doctor`, and then reports whether this machine
# can run the `process` backend (`hairspring doctor --process-backend`). That last check is information, not a failure,
# unless you pass --require-process-backend: the sim and wasm backends work everywhere the binary does.
#
# What it never does: use sudo, change kernel settings, or write outside the prefix and its own scratch directory. The
# release's SHA256SUMS comes from the same place as the binary, so it catches a corrupt or truncated download but not a
# compromised release: pass --sha256 with a hash you obtained another way to pin the binary.
#
# Test hook (used by scripts/test-scripts.sh): HAIRSPRING_RELEASE_BASE_URL replaces the release download URL.
set -euo pipefail

readonly DEFAULT_REPO="spiculedata/hairspring"
readonly EXIT_UNSUPPORTED_HOST=4
readonly LINUX_ASSET="hairspring-linux-x86_64"
readonly MACOS_ASSET="hairspring-macos-arm64"
readonly SUMS_ASSET="SHA256SUMS"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PREFIX="${HOME:-}/.local"
REPO="$DEFAULT_REPO"
VERSION=""
PINNED_SHA256=""
FROM_SOURCE="no"
WITH_UI="no"
REQUIRE_PROCESS_BACKEND="no"
DRY_RUN="no"
SCRATCH=""

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
  cat <<EOF
Usage: install.sh [--prefix DIR] [--version TAG | --from-source] [--repo OWNER/REPO] [--sha256 HEX]
                  [--with-ui] [--require-process-backend] [--dry-run]

Without --version the checkout this script lives in is built with 'cargo build --release --locked -p hairspring-cli'.
With --version the release asset for this platform is downloaded and verified before it is installed.

  --prefix DIR                Install to DIR/bin/hairspring. Default: \$HOME/.local
  --version TAG               Download release TAG (for example v0.1.0) from GitHub instead of building.
  --from-source               Build from the checkout (the default without --version).
  --repo OWNER/REPO           GitHub repository to download from. Default: $DEFAULT_REPO
  --sha256 HEX                Also require the downloaded binary to have exactly this sha256 (obtained out of band).
  --with-ui                   From source only: build the web UI first (needs node and npm), so 'hairspring serve'
                              embeds it. Without it the server shows a placeholder page.
  --require-process-backend   Exit 1 if this machine cannot run the process backend (default: report it and succeed).
  --dry-run                   Check the arguments, the platform and the tools; download and build nothing.

Release assets: $LINUX_ASSET (Linux x86-64), $MACOS_ASSET (macOS arm64) and $SUMS_ASSET.
Exit codes: 0 success, 1 a check or verification failed, 2 usage error.
EOF
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    die "neither sha256sum nor shasum is available"
  fi
}

is_hex_sha256() {
  case "$1" in
    *[!0-9a-f]*) return 1 ;;
  esac
  [ "${#1}" -eq 64 ]
}

cleanup() {
  if [ -n "$SCRATCH" ] && [ -d "$SCRATCH" ]; then
    rm -rf "$SCRATCH"
  fi
}

asset_for_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os/$arch" in
    Linux/x86_64) printf '%s' "$LINUX_ASSET" ;;
    Darwin/arm64) printf '%s' "$MACOS_ASSET" ;;
    *) return 1 ;;
  esac
}

base_url() {
  if [ -n "${HAIRSPRING_RELEASE_BASE_URL:-}" ]; then
    printf '%s' "$HAIRSPRING_RELEASE_BASE_URL"
  else
    printf 'https://github.com/%s/releases/download/%s' "$REPO" "$VERSION"
  fi
}

fetch() {
  # $1 url, $2 destination
  local protocols="=https"
  [ -z "${HAIRSPRING_RELEASE_BASE_URL:-}" ] || protocols="=https,file"
  curl --fail --silent --show-error --location --proto "$protocols" --output "$2" "$1" ||
    die "download failed: $1"
}

expected_hash_from_sums() {
  # $1 SHA256SUMS file, $2 asset name: prints the one hash listed for it
  local found count
  found="$(awk -v name="$2" '$2 == name || $2 == "*" name { print $1 }' "$1")"
  count="$(printf '%s\n' "$found" | grep -c . || true)"
  [ "$count" -eq 1 ] || die "$SUMS_ASSET lists $count entries for $2 (expected exactly one)"
  is_hex_sha256 "$found" || die "$SUMS_ASSET has a malformed hash for $2"
  printf '%s' "$found"
}

verify_staged_binary() {
  local staged="$1" version
  chmod 0755 "$staged"
  version="$("$staged" --version 2>&1)" || die "the binary does not run: '$staged --version' failed: $version"
  case "$version" in
    hairspring\ [0-9]*) ;;
    *) die "'$staged --version' did not print 'hairspring <version>': $version" ;;
  esac
  say "version: $version"
}

install_staged() {
  local staged="$1" target="$PREFIX/bin/hairspring" tmp
  mkdir -p "$PREFIX/bin"
  tmp="$PREFIX/bin/.hairspring.new.$$"
  cp "$staged" "$tmp"
  chmod 0755 "$tmp"
  mv -f "$tmp" "$target"
  say "installed: $target"
  say "sha256:    $(sha256_of "$target")"
}

stage_download() {
  local asset="$1" url sums expected actual
  command -v curl >/dev/null 2>&1 || die "curl is required to download a release (or build from source without --version)"
  url="$(base_url)"
  say "downloading $asset and $SUMS_ASSET from $url"
  fetch "$url/$SUMS_ASSET" "$SCRATCH/$SUMS_ASSET"
  fetch "$url/$asset" "$SCRATCH/$asset"
  sums="$SCRATCH/$SUMS_ASSET"
  expected="$(expected_hash_from_sums "$sums" "$asset")"
  actual="$(sha256_of "$SCRATCH/$asset")"
  if [ "$actual" != "$expected" ]; then
    die "sha256 mismatch for $asset: downloaded $actual, $SUMS_ASSET says $expected. Nothing was installed."
  fi
  if [ -n "$PINNED_SHA256" ] && [ "$actual" != "$PINNED_SHA256" ]; then
    die "sha256 of $asset is $actual, not the --sha256 you gave ($PINNED_SHA256). Nothing was installed."
  fi
  say "sha256 verified: $actual"
  STAGED="$SCRATCH/$asset"
}

stage_source_build() {
  [ -f "$REPO_ROOT/Cargo.toml" ] && [ -d "$REPO_ROOT/crates/hairspring-cli" ] ||
    die "this script is not inside a Hairspring checkout; pass --version TAG to download a release instead"
  command -v cargo >/dev/null 2>&1 || die "cargo is required to build from source (https://rustup.rs), or pass --version TAG to download a release"
  if [ "$WITH_UI" = "yes" ]; then
    command -v npm >/dev/null 2>&1 || die "--with-ui needs node and npm"
    say "building the web UI"
    (cd "$REPO_ROOT/web" && npm ci && npm run build)
  fi
  say "building hairspring (cargo build --release --locked)"
  (cd "$REPO_ROOT" && cargo build --release --locked -p hairspring-cli)
  STAGED="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/release/hairspring"
  [ -x "$STAGED" ] || die "the build finished but $STAGED does not exist"
}

run_doctor() {
  local bin="$PREFIX/bin/hairspring" work
  work="$SCRATCH/doctor"
  mkdir -p "$work"
  say "running: hairspring doctor"
  (cd "$work" && "$bin" doctor) || die "'hairspring doctor' failed"
}

report_process_backend() {
  local bin="$PREFIX/bin/hairspring" work rc=0
  work="$SCRATCH/doctor"
  mkdir -p "$work"
  say "running: hairspring doctor --process-backend"
  (cd "$work" && "$bin" doctor --process-backend) || rc=$?
  case "$rc" in
    0) say "the process backend's prerequisites are met on this machine" ;;
    "$EXIT_UNSUPPORTED_HOST")
      say "note: this machine cannot run the process backend (details above). The sim and wasm backends are not affected."
      [ "$REQUIRE_PROCESS_BACKEND" != "yes" ] || exit 1
      ;;
    *)
      warn "'hairspring doctor --process-backend' exited $rc: this build may predate that check"
      [ "$REQUIRE_PROCESS_BACKEND" != "yes" ] || exit 1
      ;;
  esac
}

parse_args() {
  while [ "$#" -gt 0 ]; do
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
      --version)
        [ "$#" -ge 2 ] || usage_error "--version needs a tag"
        VERSION="$2"
        shift 2
        ;;
      --repo)
        [ "$#" -ge 2 ] || usage_error "--repo needs OWNER/REPO"
        REPO="$2"
        shift 2
        ;;
      --sha256)
        [ "$#" -ge 2 ] || usage_error "--sha256 needs a hash"
        PINNED_SHA256="$2"
        shift 2
        ;;
      --from-source)
        FROM_SOURCE="yes"
        shift
        ;;
      --with-ui)
        WITH_UI="yes"
        shift
        ;;
      --require-process-backend)
        REQUIRE_PROCESS_BACKEND="yes"
        shift
        ;;
      --dry-run)
        DRY_RUN="yes"
        shift
        ;;
      *) usage_error "unknown option: $1" ;;
    esac
  done
}

validate_args() {
  [ -n "$PREFIX" ] || usage_error "the prefix is empty"
  if [ -n "$VERSION" ] && [ "$FROM_SOURCE" = "yes" ]; then
    usage_error "--version and --from-source cannot be combined"
  fi
  if [ -n "$VERSION" ]; then
    case "$VERSION" in
      v[0-9]*) ;;
      *) usage_error "--version must be a release tag such as v0.1.0" ;;
    esac
    case "$VERSION" in
      *[!0-9A-Za-z._-]*) usage_error "--version contains characters a tag cannot have" ;;
    esac
  fi
  case "$REPO" in
    */*/* | */ | /* | *[!0-9A-Za-z._/-]* | '') usage_error "--repo must look like OWNER/REPO" ;;
    */*) ;;
    *) usage_error "--repo must look like OWNER/REPO" ;;
  esac
  if [ -n "$PINNED_SHA256" ] && ! is_hex_sha256 "$PINNED_SHA256"; then
    usage_error "--sha256 must be 64 lowercase hex digits"
  fi
  if [ -n "$PINNED_SHA256" ] && [ -z "$VERSION" ]; then
    usage_error "--sha256 applies to a downloaded release; give --version"
  fi
  if [ "$WITH_UI" = "yes" ] && [ -n "$VERSION" ]; then
    usage_error "--with-ui applies to a source build; a release already includes the UI"
  fi
}

main() {
  local asset=""
  parse_args "$@"
  validate_args
  if [ -n "$VERSION" ]; then
    asset="$(asset_for_platform)" ||
      die "no release binary for $(uname -s) $(uname -m) (available: Linux x86-64, macOS arm64). Build from source instead: scripts/install.sh --from-source"
    say "plan: download $asset from release $VERSION of $REPO, verify its sha256, install to $PREFIX/bin"
  else
    say "plan: build this checkout with cargo (--locked) and install to $PREFIX/bin"
  fi
  if [ "$DRY_RUN" = "yes" ]; then
    if [ -n "$VERSION" ]; then
      command -v curl >/dev/null 2>&1 || die "curl is required to download a release"
    else
      command -v cargo >/dev/null 2>&1 || die "cargo is required to build from source"
    fi
    say "dry run: nothing was downloaded, built or written."
    exit 0
  fi
  SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/hairspring-install.XXXXXX")"
  trap cleanup EXIT
  STAGED=""
  if [ -n "$VERSION" ]; then
    stage_download "$asset"
  else
    stage_source_build
  fi
  verify_staged_binary "$STAGED"
  install_staged "$STAGED"
  case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) say "note: $PREFIX/bin is not on your PATH; add it to run 'hairspring' directly" ;;
  esac
  run_doctor
  report_process_backend
}

main "$@"
