#!/usr/bin/env bash
# Runs inside a fresh Ubuntu container, started by scripts/test-install-docker.sh. Exercises the install path the way a
# new user would, as an ordinary user without sudo:
#
#   1. install-hermit.sh check            must exit 0 or 1, never crash; when it exits 1 it must say CANNOT RUN
#   2. install-hermit.sh install --dry-run (with --install-toolchain)   pins verify, build prerequisites are met
#   3. install-hermit.sh install          only with HS_WITH_HERMIT_BUILD=1 (slow); HS_EMULATED=1 adds --skip-run-verify
#   4. install.sh --from-source           builds the CLI with cargo --locked, checks --version, runs doctor
#   5. hairspring doctor --process-backend  exits 0 or 4, and says CANNOT RUN when it exits 4
#
# Hermit itself cannot run in an ordinary container (no performance counters), so a container is expected to be told
# so, clearly, by steps 1 and 5. Expects the repository at /src. Environment: HS_WITH_HERMIT_BUILD, HS_SKIP_CLI_BUILD,
# HS_EMULATED (all "1" or unset).
set -euo pipefail

readonly SRC=/src
readonly TESTER=tester
readonly TESTER_HOME="/home/$TESTER"
FAILURES=0

say() { printf '%s\n' "$*"; }
pass() { say "PASS: $*"; }
fail() {
  say "FAIL: $*"
  FAILURES=$((FAILURES + 1))
}

as_tester() {
  runuser -u "$TESTER" -- env "HOME=$TESTER_HOME" \
    "PATH=$TESTER_HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" "$@"
}

prepare_image() {
  say "== preparing the container ($(. /etc/os-release && echo "$PRETTY_NAME"), $(uname -m), glibc $(ldd --version | head -n 1))"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends \
    ca-certificates curl git build-essential pkg-config libunwind-dev liblzma-dev util-linux >/dev/null
  id "$TESTER" >/dev/null 2>&1 || useradd --create-home --shell /bin/bash "$TESTER"
  chown -R "$TESTER:$TESTER" "$SRC"
  if command -v sudo >/dev/null 2>&1; then
    say "note: sudo is installed in this image; the scripts under test never call it"
  fi
}

install_rust() {
  say "== installing a stable Rust toolchain for the test user (rustup, minimal profile)"
  as_tester bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null'
  as_tester rustc --version
}

step_check() {
  say "== step 1: install-hermit.sh check"
  local out rc=0
  out="$(as_tester bash "$SRC/scripts/install-hermit.sh" check 2>&1)" || rc=$?
  say "$out"
  case "$rc" in
    0) pass "check exited 0: this container can run Hermit's prerequisites (unusual for a container)" ;;
    1)
      if printf '%s' "$out" | grep -q "CANNOT RUN"; then
        pass "check exited 1 and said CANNOT RUN"
      else
        fail "check exited 1 without a CANNOT RUN explanation"
      fi
      ;;
    *) fail "check exited $rc (expected 0 or 1)" ;;
  esac
}

step_dry_run() {
  say "== step 2: install-hermit.sh install --dry-run"
  local rc=0
  as_tester bash "$SRC/scripts/install-hermit.sh" install --dry-run --install-toolchain --prefix "$TESTER_HOME/hermit-dry" || rc=$?
  if [ "$rc" = 0 ]; then pass "dry run verified the pins and the build prerequisites"; else fail "dry run exited $rc"; fi
  if [ -e "$TESTER_HOME/hermit-dry" ]; then fail "dry run created its prefix"; fi
}

step_hermit_build() {
  [ "${HS_WITH_HERMIT_BUILD:-}" = "1" ] || {
    say "== step 3: skipped (set HS_WITH_HERMIT_BUILD=1 for the slow Hermit build)"
    return 0
  }
  say "== step 3: install-hermit.sh install (slow)"
  local prefix="$TESTER_HOME/hermit" extra=() rc=0
  [ "${HS_EMULATED:-}" != "1" ] || extra+=(--skip-run-verify)
  as_tester bash "$SRC/scripts/install-hermit.sh" install --install-toolchain --prefix "$prefix" ${extra[@]+"${extra[@]}"} || rc=$?
  if [ "$rc" != 0 ]; then
    fail "Hermit build exited $rc"
    return 0
  fi
  [ -x "$prefix/bin/hermit" ] && pass "hermit installed at $prefix/bin/hermit" || fail "no hermit binary at $prefix/bin/hermit"
  [ -f "$prefix/hermit.provenance" ] && pass "provenance recorded" || fail "no provenance file"
  say "sha256 $(sha256sum "$prefix/bin/hermit" | cut -d' ' -f1)"
  if [ "${HS_EMULATED:-}" != "1" ]; then
    as_tester "$prefix/bin/hermit" --version && pass "hermit --version ran" || fail "hermit --version failed"
  fi
}

step_cli() {
  [ "${HS_SKIP_CLI_BUILD:-}" != "1" ] || {
    say "== steps 4 and 5: skipped (HS_SKIP_CLI_BUILD=1)"
    return 0
  }
  say "== step 4: install.sh --from-source"
  local prefix="$TESTER_HOME/cli" rc=0 out
  as_tester bash "$SRC/scripts/install.sh" --from-source --prefix "$prefix" || rc=$?
  # install.sh succeeds when the CLI builds and runs, whatever the process backend's state.
  if [ "$rc" = 0 ]; then pass "install.sh built, verified and installed the CLI"; else fail "install.sh exited $rc"; return 0; fi
  as_tester "$prefix/bin/hairspring" --version && pass "hairspring --version ran" || fail "hairspring --version failed"
  say "== step 5: hairspring doctor --process-backend"
  rc=0
  out="$(as_tester "$prefix/bin/hairspring" doctor --process-backend 2>&1)" || rc=$?
  say "$out"
  case "$rc" in
    0) pass "doctor --process-backend exited 0" ;;
    4)
      if printf '%s' "$out" | grep -q "CANNOT RUN"; then
        pass "doctor --process-backend exited 4 and said CANNOT RUN"
      else
        fail "doctor --process-backend exited 4 without CANNOT RUN"
      fi
      ;;
    *) fail "doctor --process-backend exited $rc (expected 0 or 4)" ;;
  esac
}

prepare_image
install_rust
step_check
step_dry_run
step_hermit_build
step_cli
say
if [ "$FAILURES" = 0 ]; then
  say "all steps passed"
else
  say "$FAILURES step(s) failed"
  exit 1
fi
