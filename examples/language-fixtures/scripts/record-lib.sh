#!/bin/sh
# Shared parts of the build-record scripts (build-record.sh, build-record-python.sh, build-record-rust.sh). Sourced, not run.
#
# The caller sets `work` (a scratch directory) and `out` (the record file), calls record_init, runs its steps, writes the
# alignment checklist with alignment_checklist and ends with finish_record. Every step records a verdict under its id;
# a checklist line is ticked only when every step that proves it passed, and otherwise left unticked with the failure
# shown. The record is assembled at the end: title, producer fields, artifact sha256 and verdicts first, then the steps.
#
# The variables below are set by the caller or read by it, which shellcheck cannot see from here.
# shellcheck disable=SC2034,SC2154

record_init() {
  body=$work/body.md
  statusfile=$work/status.txt
  reviewfile=$work/review-needed.txt
  : >"$body"
  : >"$statusfile"
  : >"$reviewfile"
  failed=0
  # A record lists only what this run published: an old SHA256SUMS in a reused directory would name other bytes.
  if [ -n "${ARTIFACT_DIR:-}" ]; then
    mkdir -p "$ARTIFACT_DIR" && : >"$ARTIFACT_DIR/SHA256SUMS"
  fi
}

# source_commit_of <repo>: the HEAD commit, with +dirty when the tree has uncommitted or untracked changes, or unknown.
source_commit_of() {
  sha=$(git -C "$1" rev-parse HEAD 2>/dev/null) || { echo unknown; return 0; }
  if [ -n "$(git -C "$1" status --porcelain 2>/dev/null)" ]; then
    echo "$sha+dirty"
  else
    echo "$sha"
  fi
}

# equal_hashes <name> <first> <second>: the two builds must have produced a binary, and the same one.
equal_hashes() {
  echo "sha256 of $1, first build:  $2"
  echo "sha256 of $1, second build: $3"
  if [ -z "$2" ] || [ -z "$3" ]; then
    echo "FAIL a build produced no binary to hash"
    return 1
  fi
  [ "$2" = "$3" ] || { echo "the two builds differ"; return 1; }
  echo "the two builds are equal"
}

# require_lines: copies stdin to stdout and fails when there was nothing (a source excerpt that found nothing proves nothing).
require_lines() {
  shown=$(cat)
  if [ -z "$shown" ]; then
    echo "FAIL nothing to show: the source excerpt is empty"
    return 1
  fi
  printf '%s\n' "$shown"
}

say() { printf '%s\n' "$*" >>"$body"; }

# step <id> <title> <command...>: runs the command, records its output in a fenced block and PASS or FAIL under the id.
step() {
  id=$1
  title=$2
  shift 2
  log=$(mktemp "${TMPDIR:-/tmp}/record.XXXXXX")
  "$@" >"$log" 2>&1
  rc=$?
  status=PASS
  if [ "$rc" -ne 0 ]; then
    status=FAIL
    failed=1
  fi
  printf '%s %s\n' "$id" "$status" >>"$statusfile"
  say "### $id $title: $status"
  say ""
  say '```'
  say "\$ $*"
  cat "$log" >>"$body"
  say '```'
  say ""
  rm -f "$log"
  return "$rc"
}

# not_run <id> <title> <reason>: a check that could not run here. It does not pass.
not_run() {
  printf '%s NOT-RUN\n' "$1" >>"$statusfile"
  say "### $1 $2: NOT-RUN"
  say ""
  say "$3"
  say ""
}

# not_applicable <id> <title> <reason> [yes]: a check that does not apply to this language. It does not fail, and it is
# shown as N/A, never as PASS. With a fourth argument the spec asks for a code-review statement in its place: the record
# says that statement has not been made.
not_applicable() {
  printf '%s N/A\n' "$1" >>"$statusfile"
  say "### $1 $2: NOT-APPLICABLE"
  say ""
  say "$3"
  say ""
  if [ -n "${4:-}" ]; then
    printf '%s\n' "$1" >>"$reviewfile"
    say "Required by the spec and not yet made [I]: the code-review statement that stands in for $1. A reviewer writes it; this tool does not."
    say ""
  fi
}

# verdict_label <id>: like verdict, but N/A when only not-applicable rows exist and PASS+N/A when both kinds do.
verdict_label() {
  v=$(verdict "$1")
  if [ "$v" = PASS ]; then
    na=$(grep -c "^$1 N/A$" "$statusfile")
    ok=$(grep -c "^$1 PASS$" "$statusfile")
    if [ "$na" -gt 0 ] && [ "$ok" -eq 0 ]; then
      echo N/A
    elif [ "$na" -gt 0 ]; then
      echo PASS+N/A
    else
      echo PASS
    fi
  else
    echo "$v"
  fi
}

# verdict <id>: PASS (every step of that id passed or does not apply), FAIL, NOT-RUN or MISSING (no step recorded).
verdict() {
  rows=$(grep "^$1 " "$statusfile" | cut -d' ' -f2 | sort -u)
  if [ -z "$rows" ]; then
    echo MISSING
  elif printf '%s\n' "$rows" | grep -qx FAIL; then
    echo FAIL
  elif printf '%s\n' "$rows" | grep -qx NOT-RUN; then
    echo NOT-RUN
  else
    echo PASS
  fi
}

# tickline <number> <text> <id>...: a checklist line, ticked only when every proving id passed.
tickline() {
  number=$1
  text=$2
  shift 2
  bad=""
  for id in "$@"; do
    v=$(verdict "$id")
    [ "$v" = PASS ] || bad="${bad:+$bad, }$id $v"
  done
  if [ -z "$bad" ]; then
    proved=""
    for id in "$@"; do
      label=$(verdict_label "$id")
      if [ "$label" = PASS ]; then
        proved="${proved:+$proved, }$id"
      else
        proved="${proved:+$proved, }$id ($label)"
      fi
    done
    say "- [x] $number. $text (proved by $proved)"
  else
    say "- [ ] $number. $text (not ticked: $bad)"
  fi
}

# extract <regex> <objdump-file>: the disassembly of the function whose header line matches.
extract() { awk -v re="$1" '$0 ~ re { on = 1 } on { print } on && /^$/ { exit }' "$2"; }

if command -v sha256sum >/dev/null 2>&1; then
  hash_file() { sha256sum "$1" | cut -d' ' -f1; }
else
  hash_file() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

# publish_artifact <file> <name>: prints the sha256 of the file; copies it to $ARTIFACT_DIR/<name> when that is set, with
# a line in SHA256SUMS, so the workflow can upload the exact bytes the record names.
publish_artifact() {
  [ -f "$1" ] || { echo "no such artifact: $1" >&2; return 1; }
  sha=$(hash_file "$1") || return 1
  if [ -n "${ARTIFACT_DIR:-}" ]; then
    mkdir -p "$ARTIFACT_DIR" || return 1
    cp "$1" "$ARTIFACT_DIR/$2" || return 1
    printf '%s  %s\n' "$sha" "$2" >>"$ARTIFACT_DIR/SHA256SUMS" || return 1
  fi
  echo "$sha"
}

# publish_tree <dir> <name> <tree-hash>: the same for a staged package (a directory), identified by its tree hash. SHA256SUMS
# lists every file (so `sha256sum -c SHA256SUMS` works from the artifact directory); the tree hash is in <name>.tree.
publish_tree() {
  [ -d "$1" ] || { echo "no such directory: $1" >&2; return 1; }
  if [ -n "${ARTIFACT_DIR:-}" ]; then
    mkdir -p "$ARTIFACT_DIR/$2" || return 1
    cp -R "$1/." "$ARTIFACT_DIR/$2/" || return 1
    (cd "$1" && find . -type f | sed 's|^\./||' | LC_ALL=C sort) | while read -r f; do
      printf '%s  %s/%s\n' "$(hash_file "$1/$f")" "$2" "$f" >>"$ARTIFACT_DIR/SHA256SUMS"
    done
    printf '%s\n' "$3" >"$ARTIFACT_DIR/$2.tree" || return 1
  fi
  echo "$3"
}

# producer_block: who produced this record, from the GitHub Actions environment (one field per paragraph).
producer_block() {
  if [ -n "${GITHUB_RUN_ID:-}" ]; then
    echo "GitHub run: $GITHUB_RUN_ID (${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-}/actions/runs/$GITHUB_RUN_ID)"
  else
    echo "GitHub run: <TO BE FILLED: this record was not produced by a GitHub Actions run>"
  fi
  echo ""
  if [ -n "${GITHUB_WORKFLOW_REF:-}" ]; then
    workflow=${GITHUB_WORKFLOW_REF%@*}
    workflow=${workflow#"${GITHUB_REPOSITORY:-}"/}
    echo "Workflow file: $workflow, run at commit \`${GITHUB_WORKFLOW_SHA:-${GITHUB_SHA:-unknown}}\`."
  else
    echo "Workflow file: <TO BE FILLED: not produced by a GitHub Actions run>"
  fi
  echo ""
  if [ -n "${ImageOS:-}" ] && [ -n "${ImageVersion:-}" ]; then
    echo "Runner image: $ImageOS $ImageVersion (${RUNNER_OS:-unknown} ${RUNNER_ARCH:-unknown}; a hosted runner has no image digest)"
  else
    echo "Runner image: <TO BE FILLED: not a GitHub-hosted runner, or ImageOS and ImageVersion are not set>"
  fi
  echo ""
  echo "Producer: ${PRODUCER:-<TO BE FILLED: which host or image produced this record>}"
  echo ""
  echo "Source commit: \`${SOURCE_COMMIT:-unknown}\` (the commit the artifacts were built from)."
  echo ""
}

verdict_line() {
  ids=$(cut -d' ' -f1 "$statusfile" | awk '!seen[$0]++')
  line=""
  for id in $ids; do
    line="${line:+$line, }$id $(verdict_label "$id")"
  done
  echo "Verdicts: $line"
}

# finish_record: writes $out from record_title, record_generated_by, artifact_name, artifact_sha and the steps.
finish_record() {
  ticked=$(grep -c '^- \[x\]' "$body")
  {
    echo "# $record_title"
    echo ""
    echo "$record_generated_by"
    echo ""
    producer_block
    echo "Artifact: ${artifact_name:-unknown}, ${artifact_sha_label:-sha256} ${artifact_sha:-unknown}"
    echo ""
    echo "This is the sha256 of the artifact as this job uploaded it (the file or package in the run's artifact, next to this"
    echo "record), so the box gets exactly those bytes: compare \`shasum -a 256\` of the file as downloaded from the run with the"
    echo "value above. A zip download does not keep the executable bit; restore it with chmod."
    echo ""
    verdict_line
    echo ""
    echo "Checklist lines ticked: $ticked of 11."
    echo ""
    cat "$body"
  } >"$out"
  echo "wrote $out"
}

# evidence_steps: the evidence for checklist lines 2, 4, 6, 7, 8, 9 and 11, captured from the built artifact.
# Reads: here (the scripts directory), lang, evidence_artifact (a runnable program), barrier_sources (source files, space
# separated), and the function forbidden_reads (the grep of the fixture source for line 11).
evidence_steps() {
  check="sh $here/check-evidence.sh"
  sample=$work/sample
  # shellcheck disable=SC2086  # $check is a command and its argument on purpose
  $check capture "$evidence_artifact" buggy "$sample" >/dev/null 2>&1
  # shellcheck disable=SC2086
  step L2 "line 2: a bad argument and no argument each print a usage line on stderr, nothing on stdout, and exit 64" $check usage "$lang" "$evidence_artifact"
  # shellcheck disable=SC2086
  step L3 "line 3: T = 4, K = 1000, W = 1000, W_wide = 10000, read from the constants of the source" $check constants "$lang" $constant_sources
  # shellcheck disable=SC2086
  step L4 "line 4: the start barrier in the source" $check barrier $barrier_sources
  # shellcheck disable=SC2086
  step L6 "line 6: the always and sometimes event lines of one native buggy run, each evaluated once, with the witness values" $check witness "$lang" "$sample"
  # shellcheck disable=SC2086
  step L7 "line 7: the property ids in those event lines" $check ids "$lang" "$sample"
  # shellcheck disable=SC2086
  step L8 "line 8: one sample LANGAT v1 stdout line (B5 shows it is one write)" $check stdout-line "$lang" buggy "$sample"
  # shellcheck disable=SC2086
  step L9 "line 9: exit codes 0, 3 and 64" $check exit-codes "$lang" "$evidence_artifact"
  step L11 "line 11: the fixture source reads no seed, /proc, CPU count or clock (a grep of the source files)" forbidden_reads
}

# alignment_checklist <line 1 text> <line 2 text> <line 11 text>: the 11 lines of the fixture alignment checklist, ticked
# when the evidence passes. Lines 1, 3, 5 and 10 point at the B-sections that prove them.
alignment_checklist() {
  say "## Alignment checklist (ticked by this script when the evidence above passes; left open, with the failure shown, when it does not)"
  say ""
  tickline 1 "$1 (not checked by the tool: that no build tag or -D flag chooses the mode)" B1
  tickline 2 "$2" L2
  tickline 3 "T = 4, K = 1000, W = 1000, W_wide = 10000 (L3 checks the constant values in the source, not that every mode uses them; B2 shows the immediates of the window for the compiled languages); wide differs from buggy only in the loop count (B7 shows the source; not checked by the tool: that nothing else differs)." B7 B4 L3
  tickline 4 "Start barrier: each worker increments started, then waits until started == 4 (not checked by the tool: that the wait is in the same function as the increment; it checks that a wait follows the increment within six lines or in a function called there, and ignores comments and the sometimes() call; known false accept: a line such as \`if (started == 4) log();\` after the increment is taken for a wait)." L4
  tickline 5 "The window: load; spin; store; nothing else between them (B2)." B2
  tickline 6 "Main, after joining: always(final == T*K) with four witness values [final, 4000, 4, 1000], then sometimes(started == 4), each once (L6 checks that the evaluating thread's key is the key the SDK gives the main thread, t#5 (Go: go), which the workers' keys t#1 to t#4 are not; not checked by the tool: which OS thread that key belongs to)." L6 B4 L8
  tickline 7 "Ids are \`$lang.counter_exact\` and \`$lang.all_started\`." L7
  tickline 8 "One stdout line in the contract form, one write; nothing else on stdout." L8 B5
  tickline 9 "Exit status 0 whatever the property says; 3 for an internal invariant failure; 64 for a usage error." L9 L2 B4
  tenth="B1 to B7 pass and the SDK's conformance goldens pass."
  if [ -s "$reviewfile" ]; then
    tenth="$tenth (not checked by the tool: the code-review statements the spec requires in place of $(tr '\n' ' ' <"$reviewfile" | sed 's/ $//; s/ /, /g'); they are not yet made [I])."
  fi
  tickline 10 "$tenth" B1 B2 B3 B4 B5 B6 B7
  tickline 11 "$3" L11
}
