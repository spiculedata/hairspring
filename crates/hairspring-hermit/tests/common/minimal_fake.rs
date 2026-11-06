//! A fake `hermit` that builds the guest's environment the way the pinned Hermit source does
//! (`apply_base_and_explicit_environment`), so that tests see what a guest would see under each `--base-env`.
//!
//! - `--base-env` is read case-insensitively, `--env`/`-e` in every spelling, and the last `--base-env` wins.
//! - `empty` starts from nothing; `minimal` from `HOSTNAME`, `PATH` and `HOME`; `host` (and no flag) from the fake's own
//!   environment without `HERMIT_VERIFY_TMPDIR`. Any other value is refused with status 2.
//! - Each `--env` is applied in order, later entries override earlier ones. `NAME=VALUE` sets it, a bare `NAME` copies
//!   the fake's own value and fails with status 97 if there is none.
//! - `ASAN_OPTIONS` and `LSAN_OPTIONS` are forced last, in every mode.
//! - `--tmp=DIR` stands in for the mount of the guest's `/tmp`: `HAIRSPRING_EVENTS_FILE`, when the fake has one, is
//!   pointed into `DIR`.
//!
//! `MF_LOG` (set through `extra_env`, so it exists only in the fake's own environment and is removed before the guest
//! starts) names a directory. The fake writes there `argv-<seed>` (its arguments, one per line), `tmpdir-<seed>` (its
//! own `TMPDIR`, or `unset`) and `verify-tmpdir-<seed>`.
pub const FAKE: &str = r#"#!/bin/sh
[ "$1" = "--version" ] && { echo "fake-hermit 0.0"; exit 0; }
if [ -n "$MF_LOG" ]; then printf '%s\n' "$@" > "$MF_LOG/argv-pending"; fi
shift
seed=0
mode=host
tmp=
n=0
lower() { printf '%s' "$1" | tr 'A-Z' 'a-z'; }
add() { n=$((n + 1)); eval "spec_$n=\$1"; }
while [ "$1" != "--" ]; do
  case "$1" in
    --sched-seed=*) seed=${1#--sched-seed=};;
    --tmp=*) tmp=${1#--tmp=};;
    --base-env=*) mode=$(lower "${1#--base-env=}");;
    --base-env) shift; mode=$(lower "$1");;
    --env=*) add "${1#--env=}";;
    --env) shift; add "$1";;
    -e=*) add "${1#-e=}";;
    -e) shift; add "$1";;
    -e?*) add "${1#-e}";;
  esac
  shift
done
shift
if [ -n "$MF_LOG" ]; then
  mv "$MF_LOG/argv-pending" "$MF_LOG/argv-$seed"
  echo "${TMPDIR-unset}" > "$MF_LOG/tmpdir-$seed"
  echo "${HERMIT_VERIFY_TMPDIR-unset}" > "$MF_LOG/verify-tmpdir-$seed"
fi
unset MF_LOG
if [ -n "$tmp" ] && [ -n "${HAIRSPRING_EVENTS_FILE-}" ]; then
  export HAIRSPRING_EVENTS_FILE="$tmp/hairspring-events"
fi
set -- ASAN_OPTIONS=detect_leaks=0 LSAN_OPTIONS=detect_leaks=0 "$@"
i=$n
while [ "$i" -ge 1 ]; do
  eval "spec=\$spec_$i"
  case "$spec" in
    *=*) set -- "$spec" "$@";;
    *)
      if eval "[ \"\${$spec+set}\" = set ]"; then
        eval "value=\${$spec}"
        set -- "$spec=$value" "$@"
      else
        echo "Error: environment variable $spec is not set" >&2
        exit 97
      fi;;
  esac
  i=$((i - 1))
done
echo "hermit: virtual-time epoch=fake" >&2
case "$mode" in
  empty) exec /usr/bin/env -i "$@";;
  minimal)
    exec /usr/bin/env -i HOSTNAME=hermetic-container.local \
      PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root "$@";;
  host|default) exec /usr/bin/env -u HERMIT_VERIFY_TMPDIR "$@";;
  *) echo "Error: unknown base environment" >&2; exit 2;;
esac
"#;
