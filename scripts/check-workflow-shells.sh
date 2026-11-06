#!/usr/bin/env bash
# Fails when a GitHub Actions job that runs in a container does not select bash for its `run:` steps.
#
#   scripts/check-workflow-shells.sh [WORKFLOW.yml ...]     (default: .github/workflows/*.yml)
#   scripts/check-workflow-shells.sh --self-test
#
# In a container job GitHub runs `run:` steps with `sh -e {0}`, which is dash on Ubuntu images, not bash. A step that
# uses `${VAR:0:7}`, `[[ ]]`, arrays or `<<<` then fails with "Bad substitution" or a syntax error, on a runner only. The
# job must say `defaults: run: shell: bash ...` (or the same for each step) so the steps run the way they were written.
# actionlint does not catch this: it assumes bash when no shell is set.
#
# A line-based check, not a YAML parser: it expects jobs indented two spaces, `container:` at four, and a `shell: bash`
# line anywhere in the same job. Exit 0 when every container job selects bash, 1 when one does not, 2 for a usage error.
set -u

check_file() {
  awk -v file="$1" '
    function finish() {
      if (job != "" && has_container && !has_bash) {
        printf "%s: job %s runs in a container but does not select bash (add defaults: run: shell: bash -e {0})\n", file, job
        bad = 1
      }
    }
    /^jobs:/ { in_jobs = 1; next }
    in_jobs && /^[^ #]/ { finish(); job = ""; in_jobs = 0 }
    in_jobs && /^  [A-Za-z0-9_-]+:[ ]*$/ {
      finish()
      job = $1; sub(/:$/, "", job); has_container = 0; has_bash = 0; next
    }
    in_jobs && /^    container:/ { has_container = 1 }
    in_jobs && /^ +shell:[ ]+bash([ ]|$)/ { has_bash = 1 }
    END { finish(); exit bad }
  ' "$1"
}

self_test() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/workflow-shells.XXXXXX")" || return 2
  printf 'jobs:\n  a:\n    runs-on: x\n    container: ubuntu:22.04\n    steps:\n      - run: echo hi\n' > "$dir/bad.yml"
  printf 'jobs:\n  a:\n    container: ubuntu:22.04\n    defaults:\n      run:\n        shell: bash -e {0}\n  b:\n    runs-on: x\n' > "$dir/good.yml"
  printf 'jobs:\n  a:\n    container: ubuntu:22.04\n    defaults:\n      run:\n        shell: bash -e {0}\n  b:\n    container: c\n' > "$dir/second-job.yml"
  local rc=0
  check_file "$dir/good.yml" >/dev/null || { echo "self-test: good workflow rejected" >&2; rc=1; }
  check_file "$dir/bad.yml" >/dev/null && { echo "self-test: bad workflow accepted" >&2; rc=1; }
  check_file "$dir/second-job.yml" >/dev/null && { echo "self-test: bash from another job leaked in" >&2; rc=1; }
  rm -rf "$dir"
  [ "$rc" -eq 0 ] && echo "self-test passed"
  return "$rc"
}

if [ "${1:-}" = "--self-test" ]; then
  self_test
  exit $?
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "$#" -eq 0 ]; then
  set -- "$HERE"/../.github/workflows/*.yml
fi
status=0
for f in "$@"; do
  [ -f "$f" ] || { echo "no such file: $f" >&2; exit 2; }
  check_file "$f" || status=1
done
[ "$status" -eq 0 ] && echo "every container job selects bash"
exit "$status"
