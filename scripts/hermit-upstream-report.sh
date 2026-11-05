#!/usr/bin/env bash
# Report what changed upstream of the pinned Hermit build, as Markdown on stdout. Read-only.
#
#   scripts/hermit-upstream-report.sh [options]     fetch the fork and the upstream, print the report
#   scripts/hermit-upstream-report.sh --self-test   check the report against a local fixture (no network)
#
# It reads scripts/hermit.pin, clones the upstream and fetches the fork into a fresh temporary directory (partial clones:
# commits and trees, blobs only on demand), and prints: the pinned commit, the upstream commit it is based on, how far
# the pin and the fork are ahead of and behind the upstream, whether the replay patch still applies to the upstream
# head, the Rust toolchain on each side, the new upstream commits grouped by keyword, a watch list of commits in areas
# the Hermit backend depends on, open upstream issues by keyword (through `gh`, when it is available) and last-activity
# dates. The keyword groups come from commit subjects only, so treat them as a reading aid, not a classification.
#
# What it never does: write to this repository, change the pin or the patch, build Hermit, or push anything. The only
# files it writes are in its own temporary directory, which it deletes on exit. Deciding to move the pin stays a
# separate, deliberate change.
# The report is Markdown: backticks in single-quoted format strings are code spans, not command substitutions.
# shellcheck disable=SC2016
set -euo pipefail

readonly DEFAULT_UPSTREAM="https://github.com/rrnewton/hermit"
readonly DEFAULT_BRANCH="main"
readonly DEFAULT_LIMIT=25
readonly DEFAULT_ISSUE_KEYWORDS="skid hang timeout tmp kernel aarch64 java dotnet go python"
readonly DEFAULT_WATCH="skid|overshoot|pmu|rcb|preempt|timeslice|chaos|timeout|tmp|temporar|summary|cpuid|rdtsc|vdso|mountinfo|/proc|base-env|epoch|kernel"
readonly ISSUES_PER_KEYWORD=10
readonly FORK_REF="refs/hairspring-report/fork"

# Subject patterns, tried in this order on the lower-cased subject; the first match wins.
readonly GROUPS_IN_ORDER="deps build perf fix feat"
readonly RE_DEPS='^(deps|chore\(deps\))[(:! ]|(^|[^a-z])(bump|bumps|bumped|upgrade|upgrades)([^a-z]|$)|cargo\.lock|dependenc|(^|[^a-z])pin reverie|rustsec|cve-[0-9]'
readonly RE_BUILD='^(build|ci)[(:! ]|toolchain|nightly|(^|[^a-z])(compile|compiles|buck|buck2|cargo|clippy|rustfmt)([^a-z]|$)'
readonly RE_PERF='^perf[(:! ]|(^|[^a-z])(faster|speed up|speedup|slower|throughput|latency|performance)([^a-z]|$)'
readonly RE_FIX='^(fix|bugfix|hotfix)[(:! -]|(^|[^a-z])(fix|fixes|fixed|repair|repairs|regression|panic|panics|crash|crashes|hang|hangs|deadlock|leak|leaks)([^a-z]|$)'
readonly RE_FEAT='^(feat|add|adds|support|implement|introduce|allow|let|enable)[(:! ]'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIN_FILE="$SCRIPT_DIR/hermit.pin"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
UPSTREAM="$DEFAULT_UPSTREAM"
UPSTREAM_BRANCH="$DEFAULT_BRANCH"
FORK_BRANCH="$DEFAULT_BRANCH"
LIMIT="$DEFAULT_LIMIT"
ISSUE_KEYWORDS="$DEFAULT_ISSUE_KEYWORDS"
WATCH="$DEFAULT_WATCH"
CHECK_ISSUES="yes"
WORK=""

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
Usage: hermit-upstream-report.sh [options]
       hermit-upstream-report.sh --self-test

Prints a Markdown report of what changed upstream of the pinned Hermit build. Read-only: it never changes this
repository, the pin or the patch. Check what changed upstream before deciding to bump the pin.

Options:
  --pin FILE             The pin file. Default: scripts/hermit.pin next to this script.
  --repo-root DIR        Where the pin's patch path is relative to. Default: the parent of this script's directory.
  --upstream URL         The upstream repository. Default: $DEFAULT_UPSTREAM
  --upstream-branch B    The upstream branch to compare with. Default: $DEFAULT_BRANCH
  --fork-branch B        The fork's branch to compare with. Default: $DEFAULT_BRANCH
  --limit N              List at most N commits per group (counts are always complete). Default: $DEFAULT_LIMIT
  --issue-keywords "W.." Title keywords for the open-issue search. Default: "$DEFAULT_ISSUE_KEYWORDS"
  --watch REGEX          Extended regex (case-insensitive) for the watch list of commit subjects.
  --no-issues            Do not query open issues (no gh calls).
  --self-test            Run the offline self-test against a fixture repository in a temporary directory.
  -h, --help             Show this help.

Needs git; gh is optional (open issues and push dates). Exit codes: 0 report printed, 1 failure, 2 usage error.
EOF
}

cleanup() {
  if [ -n "$WORK" ] && [ -d "$WORK" ]; then rm -rf "$WORK"; fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# ---- the pin file (parsed, never sourced) -------------------------------------------------------------------------
PIN_REPO=""
PIN_COMMIT=""
PIN_PATCH_SHA256=""
PIN_PATCH_REL=""
PIN_TOOLCHAIN=""

load_pin() {
  [ -f "$PIN_FILE" ] && [ -r "$PIN_FILE" ] || die "the pin file $PIN_FILE is missing or unreadable"
  local line key value
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in '' | '#'*) continue ;; esac
    case "$line" in *=*) ;; *) die "pin file $PIN_FILE: '$line' is not a KEY=VALUE line" ;; esac
    key="${line%%=*}"
    value="${line#*=}"
    case "$key" in
      HERMIT_REPO) PIN_REPO="$value" ;;
      HERMIT_BASE_COMMIT) PIN_COMMIT="$value" ;;
      HERMIT_PATCH_SHA256) PIN_PATCH_SHA256="$value" ;;
      HERMIT_PATCH_REL) PIN_PATCH_REL="$value" ;;
      RUST_TOOLCHAIN) PIN_TOOLCHAIN="$value" ;;
      *) die "pin file $PIN_FILE: $key is not a known pin" ;;
    esac
  done <"$PIN_FILE"
  [[ "$PIN_REPO" =~ ^(https|file)://[A-Za-z0-9._/-]+$ ]] || die "pin file: HERMIT_REPO is missing or malformed"
  [[ "$PIN_COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "pin file: HERMIT_BASE_COMMIT is missing or malformed"
  [[ "$PIN_PATCH_SHA256" =~ ^[0-9a-f]{64}$ ]] || die "pin file: HERMIT_PATCH_SHA256 is missing or malformed"
  [[ "$PIN_PATCH_REL" =~ ^[A-Za-z0-9._/-]+$ ]] && [[ "$PIN_PATCH_REL" != /* ]] && [[ "$PIN_PATCH_REL" != *..* ]] ||
    die "pin file: HERMIT_PATCH_REL is missing or malformed"
  [[ "$PIN_TOOLCHAIN" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] || die "pin file: RUST_TOOLCHAIN is missing or malformed"
}

# ---- fetching -----------------------------------------------------------------------------------------------------
G() { git -C "$WORK/repo.git" "$@"; }

fetch_sources() {
  git clone --quiet --bare --filter=blob:none --single-branch --branch "$UPSTREAM_BRANCH" "$UPSTREAM" "$WORK/repo.git" ||
    die "could not clone $UPSTREAM (branch $UPSTREAM_BRANCH)"
  G fetch --quiet --no-tags --filter=blob:none "$PIN_REPO" "+refs/heads/$FORK_BRANCH:$FORK_REF" ||
    die "could not fetch $PIN_REPO (branch $FORK_BRANCH)"
  if ! G cat-file -e "$PIN_COMMIT^{commit}" 2>/dev/null; then
    G fetch --quiet --no-tags --filter=blob:none "$PIN_REPO" "$PIN_COMMIT" 2>/dev/null ||
      die "the pinned commit $PIN_COMMIT is in neither $UPSTREAM ($UPSTREAM_BRANCH) nor $PIN_REPO ($FORK_BRANCH)"
  fi
}

# ---- report pieces ------------------------------------------------------------------------------------------------
UP_REF=""

commit_line() { G log -1 --format='`%h` %cs %s' "$1"; }

ahead_behind() {
  # $1 left, $2 right: prints "A ahead / B behind" of left relative to right.
  local counts
  counts="$(G rev-list --left-right --count "$1...$2")"
  printf '%s ahead / %s behind' "${counts%%[[:space:]]*}" "${counts##*[[:space:]]}"
}

upstream_toolchain() {
  G show "$UP_REF:rust-toolchain.toml" 2>/dev/null | sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

latest_version_tag() {
  # The highest v<number> tag anywhere in the upstream (release tags need not be on the compared branch), its date,
  # and whether the compared branch contains it.
  local tag where="not on $UPSTREAM_BRANCH"
  tag="$(git ls-remote --tags --refs "$UPSTREAM" 'v[0-9]*' 2>/dev/null | sed 's#.*refs/tags/##' | sort -V | tail -n 1)"
  [ -n "$tag" ] || return 0
  [[ "$tag" =~ ^v[0-9A-Za-z._-]+$ ]] || return 0
  G fetch --quiet --no-tags --filter=blob:none "$UPSTREAM" "+refs/tags/$tag:refs/tags/$tag" 2>/dev/null || {
    printf '%s (could not fetch it)' "$tag"
    return 0
  }
  if G merge-base --is-ancestor "refs/tags/$tag" "$UP_REF" 2>/dev/null; then where="on $UPSTREAM_BRANCH"; fi
  printf '%s (%s, %s)' "$tag" "$(G for-each-ref --format='%(creatordate:short)' "refs/tags/$tag")" "$where"
}

report_pin_and_upstream() {
  local base patch_state="missing" up_tc in_upstream="no" tag
  base="$(G merge-base "$PIN_COMMIT" "$UP_REF" 2>/dev/null || true)"
  if G merge-base --is-ancestor "$PIN_COMMIT" "$UP_REF" 2>/dev/null; then in_upstream="yes"; fi
  if [ -f "$REPO_ROOT/$PIN_PATCH_REL" ]; then
    if [ "$(sha256_of "$REPO_ROOT/$PIN_PATCH_REL")" = "$PIN_PATCH_SHA256" ]; then patch_state="sha256 matches the pin"; else patch_state="sha256 DOES NOT match the pin"; fi
  fi
  up_tc="$(upstream_toolchain)"
  printf '## Pin\n\n| Item | Value |\n|---|---|\n'
  printf '| Fork | %s (branch %s) |\n' "$PIN_REPO" "$FORK_BRANCH"
  printf '| Pinned commit | %s |\n' "$(commit_line "$PIN_COMMIT")"
  printf '| Pinned commit (full) | `%s` |\n' "$PIN_COMMIT"
  printf '| Patch | `%s`: %s |\n' "$PIN_PATCH_REL" "$patch_state"
  printf '| Rust toolchain | pin %s; upstream head %s |\n\n' "$PIN_TOOLCHAIN" "${up_tc:-not found}"
  printf '## Upstream\n\n| Item | Value |\n|---|---|\n'
  printf '| Upstream | %s (branch %s) |\n' "$UPSTREAM" "$UPSTREAM_BRANCH"
  printf '| Upstream head | %s |\n' "$(commit_line "$UP_REF")"
  if [ -n "$base" ]; then
    printf '| Upstream base of the pin (merge base) | %s |\n' "$(commit_line "$base")"
  else
    printf '| Upstream base of the pin (merge base) | none: no shared history |\n'
  fi
  printf '| Pinned commit is in upstream history | %s |\n' "$in_upstream"
  printf '| Pin vs upstream head | %s |\n' "$(ahead_behind "$PIN_COMMIT" "$UP_REF")"
  printf '| Fork branch vs upstream head | %s |\n' "$(ahead_behind "$FORK_REF" "$UP_REF")"
  tag="$(latest_version_tag)"
  printf '| Latest upstream version tag | %s |\n' "${tag:-none}"
  printf '| Last upstream commit | %s |\n' "$(G log -1 --format=%cs "$UP_REF")"
  printf '| Last fork commit | %s |\n' "$(G log -1 --format=%cs "$FORK_REF")"
  printf '\n'
  BASE="$base"
}

patch_paths() {
  sed -n 's#^+++ b/\(.*\)$#\1#p; s#^--- a/\(.*\)$#\1#p' "$1" | sort -u
}

report_patch() {
  local patch="$REPO_ROOT/$PIN_PATCH_REL" dir="$WORK/patch-check" path
  printf '## The patch against the upstream head\n\n'
  if [ ! -f "$patch" ]; then
    printf 'Not checked: %s does not exist.\n\n' "$PIN_PATCH_REL"
    return 0
  fi
  mkdir -p "$dir"
  git -C "$dir" init --quiet
  while IFS= read -r path; do
    case "$path" in /* | *..*) die "the patch names an unsafe path: $path" ;; esac
    mkdir -p "$dir/$(dirname "$path")"
    G show "$UP_REF:$path" >"$dir/$path" 2>/dev/null || rm -f "$dir/$path"
  done < <(patch_paths "$patch")
  if git -C "$dir" apply --check "$patch" 2>"$WORK/apply.err"; then
    printf 'The patch applies cleanly to the upstream head (`git apply --check`; not built, not run).\n\n'
  else
    printf 'The patch does NOT apply to the upstream head. First errors:\n\n```\n'
    head -n 5 "$WORK/apply.err"
    printf '```\n\n'
  fi
}

classify() {
  # $1 the lower-cased subject
  local subject="$1" group
  for group in $GROUPS_IN_ORDER; do
    case "$group" in
      deps) [[ "$subject" =~ $RE_DEPS ]] && { printf 'deps'; return; } ;;
      build) [[ "$subject" =~ $RE_BUILD ]] && { printf 'build'; return; } ;;
      perf) [[ "$subject" =~ $RE_PERF ]] && { printf 'perf'; return; } ;;
      fix) [[ "$subject" =~ $RE_FIX ]] && { printf 'fix'; return; } ;;
      feat) [[ "$subject" =~ $RE_FEAT ]] && { printf 'feat'; return; } ;;
    esac
  done
  printf 'other'
}

print_group() {
  # $1 title, $2 file of "hash|date|subject" lines
  local n
  n="$(wc -l <"$2" | tr -d ' ')"
  printf '### %s (%s)\n\n' "$1" "$n"
  if [ "$n" = "0" ]; then
    printf 'None.\n\n'
    return 0
  fi
  head -n "$LIMIT" "$2" | while IFS='|' read -r hash date subject; do
    printf -- '- `%s` %s %s\n' "$hash" "$date" "$subject"
  done
  [ "$n" -le "$LIMIT" ] || printf -- '- ... and %s more\n' "$((n - LIMIT))"
  printf '\n'
}

report_commits() {
  local range="$UP_REF" all="$WORK/commits.txt" lower="$WORK/subjects-lower.txt" line subject group total
  [ -z "$BASE" ] || range="$BASE..$UP_REF"
  G log --no-merges --format='%h|%cs|%s' "$range" >"$all"
  G log --no-merges --format='%s' "$range" | tr '[:upper:]' '[:lower:]' >"$lower"
  total="$(wc -l <"$all" | tr -d ' ')"
  for group in $GROUPS_IN_ORDER other watch; do : >"$WORK/group-$group.txt"; done
  while IFS= read -r line && IFS= read -r subject <&3; do
    group="$(classify "$subject")"
    printf '%s\n' "$line" >>"$WORK/group-$group.txt"
  done <"$all" 3<"$lower"
  grep -iE -- "$WATCH" "$all" >"$WORK/group-watch.txt" || true
  printf '## New upstream commits since the base (%s, merges excluded)\n\n' "$total"
  printf 'Grouped by keywords in the subject only (first match wins: deps, build, perf, fix, feat). Read the commits '
  printf 'before relying on a group.\n\n'
  print_group "fix" "$WORK/group-fix.txt"
  print_group "feat" "$WORK/group-feat.txt"
  print_group "perf" "$WORK/group-perf.txt"
  print_group "build" "$WORK/group-build.txt"
  print_group "deps" "$WORK/group-deps.txt"
  printf '### other (%s, not listed)\n\n' "$(wc -l <"$WORK/group-other.txt" | tr -d ' ')"
  printf 'Watch list: subjects matching `%s` (case-insensitive), any group.\n\n' "$WATCH"
  print_group "watch list" "$WORK/group-watch.txt"
}

github_slug() {
  # https://github.com/OWNER/REPO[.git] -> OWNER/REPO; anything else -> empty
  local rest="${1#https://github.com/}"
  [ "$rest" != "$1" ] || return 0
  rest="${rest%.git}"
  [[ "$rest" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] && printf '%s' "$rest"
}

report_issues() {
  local slug keyword pushed
  printf '## Open upstream issues\n\n'
  if [ "$CHECK_ISSUES" != "yes" ]; then
    printf 'Not checked (--no-issues).\n\n'
    return 0
  fi
  slug="$(github_slug "$UPSTREAM")"
  if [ -z "$slug" ] || ! command -v gh >/dev/null 2>&1; then
    printf 'Not checked: needs gh and a github.com upstream.\n\n'
    return 0
  fi
  pushed="$(gh api "repos/$slug" --jq '.pushed_at' 2>/dev/null || true)"
  printf 'Upstream last push (GitHub): %s\n\n' "${pushed:-unknown}"
  for keyword in $ISSUE_KEYWORDS; do
    printf '### "%s" in the title\n\n' "$keyword"
    if ! gh issue list -R "$slug" --state open --search "$keyword in:title" --limit "$ISSUES_PER_KEYWORD" \
      --json number,title,updatedAt --jq '.[] | "- #\(.number) (updated \(.updatedAt[0:10])) \(.title)"' >"$WORK/issues.txt" 2>/dev/null; then
      printf 'Query failed.\n\n'
      continue
    fi
    if [ -s "$WORK/issues.txt" ]; then cat "$WORK/issues.txt"; else printf 'None.\n'; fi
    printf '\n'
  done
}

run_report() {
  command -v git >/dev/null 2>&1 || die "git is required"
  load_pin
  WORK="$(mktemp -d "${TMPDIR:-/tmp}/hermit-upstream-report.XXXXXX")"
  trap cleanup EXIT
  fetch_sources
  UP_REF="refs/heads/$UPSTREAM_BRANCH"
  BASE=""
  printf '# Hermit upstream report\n\n'
  printf 'Generated %s by `scripts/hermit-upstream-report.sh` from `%s`. Read-only: nothing in the repository, the ' \
    "$(date -u +%Y-%m-%dT%H:%MZ)" "$(basename "$PIN_FILE")"
  printf 'pin or the patch was changed, and nothing was built or run.\n\n'
  report_pin_and_upstream
  report_patch
  report_commits
  report_issues
}

# ---- self-test (offline) ------------------------------------------------------------------------------------------
ST_FAILS=0
st_expect() {
  # $1 description, $2 fixed string expected in $3 (file)
  if grep -qF -- "$2" "$3"; then printf 'ok   %s\n' "$1"; else
    printf 'FAIL %s: missing "%s"\n' "$1" "$2"
    ST_FAILS=$((ST_FAILS + 1))
  fi
}
st_reject() {
  if grep -qF -- "$2" "$3"; then
    printf 'FAIL %s: unexpected "%s"\n' "$1" "$2"
    ST_FAILS=$((ST_FAILS + 1))
  else printf 'ok   %s\n' "$1"; fi
}

st_commit() {
  # $1 repo, $2 subject
  git -C "$1" add -A
  git -C "$1" commit --quiet --allow-empty -m "$2"
}

build_fixture() {
  local f="$1" up="$1/upstream" fork="$1/fork" root="$1/root"
  mkdir -p "$up/detcore/src" "$root/patches"
  git -C "$up" init --quiet --initial-branch=main
  git -C "$up" config uploadpack.allowFilter true
  printf 'line one\nline two\nline three\n' >"$up/detcore/src/lib.rs"
  printf '[toolchain]\nchannel = "nightly-2000-01-01"\n' >"$up/rust-toolchain.toml"
  st_commit "$up" "Initial import"
  st_commit "$up" "Bound the scheduler's first slice"
  git -C "$up" rev-parse HEAD >"$f/pin.sha"
  git clone --quiet "$up" "$fork"
  git -C "$fork" config uploadpack.allowFilter true
  st_commit "$fork" "Fork-only: carry a local note"
  # The replay patch: one changed line against the pinned commit.
  printf 'line one\nline two, patched\nline three\n' >"$up/detcore/src/lib.rs"
  git -C "$up" diff >"$root/patches/replay.diff"
  git -C "$up" checkout --quiet -- detcore/src/lib.rs
  # A release tag on a side branch, higher than the tag on main.
  git -C "$up" checkout --quiet -b release
  st_commit "$up" "Release 10.0.0"
  git -C "$up" tag v10.0.0
  git -C "$up" checkout --quiet main
  printf 'other.rs\n' >"$up/other.rs"
  st_commit "$up" "fix: stop a scheduler hang at exit"
  st_commit "$up" "Repair the timer after a handler return"
  st_commit "$up" "feat: a --chaos-draw flag"
  st_commit "$up" "Add a per-run summary field"
  st_commit "$up" "perf: faster syscall dispatch"
  st_commit "$up" "build: move to a newer toolchain"
  st_commit "$up" "Bump the reverie pin"
  st_commit "$up" "Document the user guide"
  git -C "$up" tag v9.9.9
  printf '[toolchain]\nchannel = "nightly-2001-02-03"\n' >"$up/rust-toolchain.toml"
  st_commit "$up" "Tidy the toolchain file"
  {
    printf 'HERMIT_REPO=file://%s\n' "$fork"
    printf 'HERMIT_BASE_COMMIT=%s\n' "$(cat "$f/pin.sha")"
    printf 'HERMIT_PATCH_SHA256=%s\n' "$(sha256_of "$root/patches/replay.diff")"
    printf 'HERMIT_PATCH_REL=patches/replay.diff\n'
    printf 'RUST_TOOLCHAIN=nightly-2000-01-01\n'
  } >"$f/hermit.pin"
}

self_test() {
  local f out pin_before pin
  # A fixed, minimal git configuration: the user's own (signing, URL rewrites) must not change the result.
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  export GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid
  export GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid
  f="$(mktemp -d "${TMPDIR:-/tmp}/hermit-upstream-selftest.XXXXXX")"
  WORK="$f"
  trap cleanup EXIT
  build_fixture "$f"
  pin="$(cat "$f/pin.sha")"
  pin_before="$(sha256_of "$f/hermit.pin")"
  out="$f/report.md"
  bash "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" --pin "$f/hermit.pin" --repo-root "$f/root" \
    --upstream "file://$f/upstream" --no-issues --limit 1 >"$out" 2>"$f/report.err" || {
    cat "$f/report.err" >&2
    die "self-test: the report failed"
  }
  st_expect "names the full pinned commit" "| Pinned commit (full) | \`$pin\` |" "$out"
  st_expect "pin is in upstream history" "| Pinned commit is in upstream history | yes |" "$out"
  st_expect "pin is 9 behind" "| Pin vs upstream head | 0 ahead / 9 behind |" "$out"
  st_expect "fork is 1 ahead, 9 behind" "| Fork branch vs upstream head | 1 ahead / 9 behind |" "$out"
  st_expect "base is the pinned commit" "| Upstream base of the pin (merge base) | \`${pin:0:7}" "$out"
  st_expect "toolchains on both sides" "| Rust toolchain | pin nightly-2000-01-01; upstream head nightly-2001-02-03 |" "$out"
  st_expect "the highest version tag wins, off the branch too" "| Latest upstream version tag | v10.0.0 (" "$out"
  st_expect "an off-branch tag is marked" "not on main) |" "$out"
  st_expect "patch hash checked" "sha256 matches the pin" "$out"
  st_expect "patch applies" "The patch applies cleanly to the upstream head" "$out"
  st_expect "nine new commits" "## New upstream commits since the base (9, merges excluded)" "$out"
  st_expect "fix group counted" "### fix (2)" "$out"
  st_expect "fix group lists the newest fix" "Repair the timer after a handler return" "$out"
  st_expect "limit truncates" "- ... and 1 more" "$out"
  st_expect "feat group counted" "### feat (2)" "$out"
  st_expect "perf group" "perf: faster syscall dispatch" "$out"
  st_expect "build group counted" "### build (2)" "$out"
  st_expect "deps group" "Bump the reverie pin" "$out"
  st_expect "other counted" "### other (1, not listed)" "$out"
  st_expect "watch list catches the chaos flag and the summary field" "### watch list (2)" "$out"
  st_expect "issues skipped offline" "Not checked (--no-issues)." "$out"
  st_reject "pre-pin commits are not listed" "Initial import" "$out"
  if [ "$(sha256_of "$f/hermit.pin")" = "$pin_before" ]; then printf 'ok   pin file unchanged\n'; else
    printf 'FAIL pin file changed\n'
    ST_FAILS=$((ST_FAILS + 1))
  fi

  # An upstream change to the patched line: the patch must be reported as not applying.
  printf 'line one\nline two, rewritten upstream\nline three\n' >"$f/upstream/detcore/src/lib.rs"
  st_commit "$f/upstream" "Rewrite line two"
  bash "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" --pin "$f/hermit.pin" --repo-root "$f/root" \
    --upstream "file://$f/upstream" --no-issues >"$out" 2>"$f/report.err" || die "self-test: the second report failed"
  st_expect "a conflicting upstream change is reported" "The patch does NOT apply to the upstream head" "$out"

  # A malformed pin is refused.
  sed 's/^HERMIT_BASE_COMMIT=.*/HERMIT_BASE_COMMIT=not-a-sha/' "$f/hermit.pin" >"$f/bad.pin"
  if bash "$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")" --pin "$f/bad.pin" --repo-root "$f/root" \
    --upstream "file://$f/upstream" --no-issues >/dev/null 2>"$f/bad.err"; then
    printf 'FAIL a malformed pin was accepted\n'
    ST_FAILS=$((ST_FAILS + 1))
  else
    st_expect "a malformed pin is refused" "HERMIT_BASE_COMMIT is missing or malformed" "$f/bad.err"
  fi

  if [ "$ST_FAILS" -ne 0 ]; then die "self-test: $ST_FAILS check(s) failed"; fi
  printf 'self-test passed\n'
}

# ---- main ---------------------------------------------------------------------------------------------------------
MODE="report"
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --self-test)
      MODE="self-test"
      shift
      ;;
    --no-issues)
      CHECK_ISSUES="no"
      shift
      ;;
    --pin | --repo-root | --upstream | --upstream-branch | --fork-branch | --limit | --issue-keywords | --watch)
      [ "$#" -ge 2 ] || usage_error "$1 needs a value"
      case "$1" in
        --pin) PIN_FILE="$2" ;;
        --repo-root) REPO_ROOT="$2" ;;
        --upstream) UPSTREAM="$2" ;;
        --upstream-branch) UPSTREAM_BRANCH="$2" ;;
        --fork-branch) FORK_BRANCH="$2" ;;
        --limit) LIMIT="$2" ;;
        --issue-keywords) ISSUE_KEYWORDS="$2" ;;
        --watch) WATCH="$2" ;;
      esac
      shift 2
      ;;
    *) usage_error "unknown option: $1" ;;
  esac
done
case "$LIMIT" in '' | *[!0-9]* | 0) usage_error "--limit needs a positive number" ;; esac
[[ "$UPSTREAM" =~ ^(https|file)://[A-Za-z0-9._/-]+$ ]] || usage_error "--upstream must be an https:// or file:// URL"
[[ "$UPSTREAM_BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || usage_error "--upstream-branch is not a branch name"
[[ "$FORK_BRANCH" =~ ^[A-Za-z0-9._/-]+$ ]] || usage_error "--fork-branch is not a branch name"

case "$MODE" in
  self-test) self_test ;;
  report) run_report ;;
esac
