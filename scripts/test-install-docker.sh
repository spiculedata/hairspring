#!/usr/bin/env bash
# Clean-machine test of the install path: runs scripts/test-install-in-container.sh in fresh Ubuntu containers.
#
#   scripts/test-install-docker.sh                          ubuntu:24.04 and ubuntu:22.04, linux/amd64, CLI build only
#   scripts/test-install-docker.sh --with-hermit-build      also build Hermit from source (slow; use a native x86-64 host)
#   scripts/test-install-docker.sh --distro ubuntu:24.04    one image
#   scripts/test-install-docker.sh --skip-cli-build         only the install-hermit.sh checks
#   scripts/test-install-docker.sh --dry-run                print the plan; start nothing
#
# The repository is streamed into each container as a tar archive (without target/ or .git), so the container never
# sees your build products and nothing in the container can write back to your tree. linux/amd64 on an arm64 host runs
# emulated: the builds work, but Hermit cannot start there (it needs close_range), so the Hermit build is then
# installed with --skip-run-verify and reported as unverified.
# Hermit cannot run in an ordinary container in any case (no performance counters); this test checks that it is told so.
set -euo pipefail

readonly DEFAULT_DISTROS="ubuntu:24.04 ubuntu:22.04"
readonly DEFAULT_PLATFORM="linux/amd64"
# Runs in the container: unpack the archive from stdin, then the in-container test.
readonly CONTAINER_COMMAND='mkdir -p /src && tar -xf - -C /src && exec bash /src/scripts/test-install-in-container.sh'

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
PLATFORM="$DEFAULT_PLATFORM"
WITH_HERMIT_BUILD="0"
SKIP_CLI_BUILD="0"
DRY_RUN="no"
DISTROS=()

usage_error() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}

usage() {
  sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --distro)
      [ "$#" -ge 2 ] || usage_error "--distro needs an image"
      DISTROS+=("$2")
      shift 2
      ;;
    --platform)
      [ "$#" -ge 2 ] || usage_error "--platform needs a value such as linux/amd64"
      PLATFORM="$2"
      shift 2
      ;;
    --with-hermit-build)
      WITH_HERMIT_BUILD="1"
      shift
      ;;
    --skip-cli-build)
      SKIP_CLI_BUILD="1"
      shift
      ;;
    --dry-run)
      DRY_RUN="yes"
      shift
      ;;
    *) usage_error "unknown option: $1" ;;
  esac
done

if [ "${#DISTROS[@]}" -eq 0 ]; then
  read -r -a DISTROS <<<"$DEFAULT_DISTROS"
fi

emulated="0"
case "$PLATFORM/$(uname -m)" in
  linux/amd64/x86_64) ;;
  linux/amd64/*) emulated="1" ;;
esac

run_one() {
  local distro="$1"
  tar -C "$REPO" --exclude=./target --exclude=./.git --exclude=./node_modules --exclude=./web/node_modules -cf - . |
    docker run --rm -i --platform "$PLATFORM" \
      -e "HS_WITH_HERMIT_BUILD=$WITH_HERMIT_BUILD" -e "HS_SKIP_CLI_BUILD=$SKIP_CLI_BUILD" -e "HS_EMULATED=$emulated" \
      "$distro" bash -c "$CONTAINER_COMMAND"
}

printf 'plan: %s on %s (emulated: %s); Hermit build: %s; CLI build: %s\n' \
  "${DISTROS[*]}" "$PLATFORM" "$emulated" "$WITH_HERMIT_BUILD" "$([ "$SKIP_CLI_BUILD" = 1 ] && echo skipped || echo yes)"
if [ "$DRY_RUN" = "yes" ]; then
  printf 'dry run: no container was started.\n'
  exit 0
fi
command -v docker >/dev/null 2>&1 || {
  printf 'error: docker is required\n' >&2
  exit 1
}

failed=0
for distro in "${DISTROS[@]}"; do
  printf '\n===== %s =====\n' "$distro"
  if run_one "$distro"; then
    printf 'RESULT %s: pass\n' "$distro"
  else
    printf 'RESULT %s: FAIL\n' "$distro"
    failed=1
  fi
done
exit "$failed"
