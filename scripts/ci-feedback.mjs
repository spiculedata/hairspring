#!/usr/bin/env node
// CI feedback for the rework loop: when a pull request's checks go red, leave ONE
// comment saying which jobs failed and the first error lines of each.
//
// Why this exists. A bot PR goes red, a watcher adds `needs-rework`, and the
// quality agent is meant to fix it. But the agent only sees what a pull request
// shows it: its description, comments and reviews. `ci` is a rollup that prints
// "e2e finished as failure" and nothing else, and the agent's own token cannot
// read check runs or Actions logs (HTTP 403). So it saw a red PR with no reason
// and asked a human what to change. A comment is the one channel every
// agent and every human already reads, so the reason goes there.
//
// Properties that matter more than the formatting:
//
//   1. It never executes pull request code and never checks it out. It reads the
//      Actions API (run, job and log text) and writes one comment. The workflow
//      that runs it is `workflow_run`, which runs from the default branch.
//   2. It is advisory. It cannot block or approve anything.
//   3. Log text is treated as untrusted. It goes inside a code fence that is
//      longer than any backtick run in it, so a log line cannot break out into
//      markdown, and `@mentions` inside a fence do not notify anyone.
//   4. One comment per pull request, updated in place, marked with MARKER. When
//      the checks later pass, the same comment is edited to say so, so a stale
//      "failed" never outlives the failure.
//   5. The logic is pure and takes `gh` as a function, so it is tested offline.

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const MARKER = "<!-- ci-feedback -->";

const FAILED = new Set(["failure", "timed_out", "startup_failure"]);
const WATCHED_WORKFLOWS = new Set(["CI", "Coverage gate"]);
const MAX_LINES_PER_JOB = 30;
const MAX_LINE_LENGTH = 300;
const TAIL_LINES = 15;
// GitHub rejects a comment over 65,536 characters; leave room for the frame.
const MAX_COMMENT_LENGTH = 60000;

const ANSI = /\u001b\[[0-9;]*[A-Za-z]/g;
const TIMESTAMP = /^﻿?\d{4}-\d\d-\d\dT[\d:.]+Z ?/;
// What marks a line worth showing: Actions' own error annotation, rustc and
// cargo errors, rustfmt diffs, test-runner failures and panics.
const SIGNAL =
  /(##\[error\]|(^|\s)error(\[E\d+\])?:|panicked at|^Diff in |\bFAILED\b|^\s*FAIL\s|\bAssertionError\b|✗|×|\bnot ok\b)/;
// "Process completed with exit code 1" is on every failed job and says nothing.
const GENERIC = /^##\[error\]Process completed with exit code \d+\.?$/;

/** One log line, cleaned for reading: no colour codes, no timestamp, bounded. */
export function cleanLine(line) {
  const text = line.replace(ANSI, "").replace(TIMESTAMP, "").trimEnd();
  return text.length > MAX_LINE_LENGTH ? `${text.slice(0, MAX_LINE_LENGTH)}…` : text;
}

/** The lines of a job log that explain why it failed, in log order. */
export function excerpt(log) {
  const lines = String(log ?? "")
    .split("\n")
    .map(cleanLine)
    .filter((l) => l.trim() !== "");
  const signal = [];
  for (const l of lines) {
    if (SIGNAL.test(l) && !signal.includes(l)) signal.push(l);
  }
  const useful = signal.filter((l) => !GENERIC.test(l));
  if (useful.length > 0) return useful.slice(0, MAX_LINES_PER_JOB);
  if (signal.length > 0) return signal.slice(0, MAX_LINES_PER_JOB);
  return lines.slice(-TAIL_LINES);
}

/** A code fence longer than any run of backticks inside the text. */
export function fence(text) {
  const longest = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  const ticks = "`".repeat(Math.max(3, longest + 1));
  return `${ticks}\n${text}\n${ticks}`;
}

function shortSha(sha) {
  return sha.slice(0, 7);
}

/** The comment for a red pull request. `failures` is what `collect` returns. */
export function buildComment({ sha, failures }) {
  const head = [
    MARKER,
    `## CI failed on \`${shortSha(sha)}\``,
    "",
    "Why each failing job failed, from its log (first error lines only; open the job for the rest).",
    "Fix these, then push. This comment updates itself when the checks pass.",
    "",
  ];
  const omitted = "_More failing jobs were left out to keep this comment short. Open the run for all of them._";
  let body = head.join("\n");
  let dropped = false;
  for (const run of failures) {
    for (const job of run.jobs) {
      const section = [
        `### ${job.name}`,
        `${run.workflow} · [job log](${job.url}) · [run](${run.url})`,
        "",
        fence(job.lines.length > 0 ? job.lines.join("\n") : "(no log lines were available)"),
        "",
      ].join("\n");
      if (body.length + section.length + omitted.length > MAX_COMMENT_LENGTH) {
        dropped = true;
        break;
      }
      body += `\n${section}`;
    }
    if (dropped) break;
  }
  if (failures.every((run) => run.jobs.length === 0)) {
    body += "\nNo individual failing job was found; open the failing run for the reason.\n";
    for (const run of failures) body += `\n- ${run.workflow}: ${run.url}`;
  }
  return dropped ? `${body}\n${omitted}\n` : `${body}\n`;
}

/** The comment that replaces the failure once everything passes. */
export function buildResolved({ sha }) {
  return `${MARKER}\n## CI is green on \`${shortSha(sha)}\`\n\nThe earlier failure on this pull request is fixed.\n`;
}

/**
 * One job's log. `gh api` refuses to print a response that carries terminal escape
 * sequences, and Actions logs are full of colour codes, so the flag is required; an
 * older `gh` without it falls back to the plain call.
 */
export function fetchLog({ gh, repo, jobId }) {
  const path = `repos/${repo}/actions/jobs/${jobId}/logs`;
  try {
    return gh(["api", "--allow-escape-sequences", path]);
  } catch {
    return gh(["api", path]);
  }
}

/**
 * What failed for this commit. Only the newest run of each workflow counts, so a
 * failure that a later run already replaced does not keep the comment red.
 * `pending` is true while any of those runs is still going.
 */
export function collect({ gh, repo, sha }) {
  const all = JSON.parse(gh(["api", `repos/${repo}/actions/runs?head_sha=${sha}&per_page=100`])).workflow_runs ?? [];
  const newest = new Map();
  for (const run of all) {
    if (!WATCHED_WORKFLOWS.has(run.name)) continue;
    const seen = newest.get(run.name);
    if (!seen || Date.parse(run.created_at) > Date.parse(seen.created_at)) newest.set(run.name, run);
  }
  const runs = [...newest.values()];
  const pending = runs.some((run) => run.status !== "completed");
  const failures = [];
  for (const run of runs) {
    if (!FAILED.has(run.conclusion)) continue;
    const jobs = JSON.parse(gh(["api", `repos/${repo}/actions/runs/${run.id}/jobs?per_page=100`])).jobs ?? [];
    const failedJobs = jobs
      .filter((job) => FAILED.has(job.conclusion))
      .map((job) => {
        let log = "";
        try {
          log = fetchLog({ gh, repo, jobId: job.id });
        } catch {
          // Logs expire or are not ready yet; the job link still works.
        }
        return { name: job.name, url: job.html_url, lines: excerpt(log) };
      });
    failures.push({ workflow: run.name, url: run.html_url, jobs: failedJobs });
  }
  return { failures, pending };
}

/** The id of this pull request's feedback comment, or null. */
export function findComment({ gh, repo, pr }) {
  const ids = gh([
    "api",
    `repos/${repo}/issues/${pr}/comments?per_page=100`,
    "--paginate",
    "--jq",
    `.[] | select(.body | startswith("${MARKER}")) | .id`,
  ])
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
  return ids[0] ?? null;
}

/** Create the comment, or edit it in place when one already exists. */
export function upsert({ gh, repo, pr, body, existing }) {
  if (existing) {
    gh(["api", "--method", "PATCH", `repos/${repo}/issues/comments/${existing}`, "-f", `body=${body}`]);
    return "updated";
  }
  gh(["api", "--method", "POST", `repos/${repo}/issues/${pr}/comments`, "-f", `body=${body}`]);
  return "created";
}

/** Decide and act. Returns what it did, for the log line and the tests. */
export function run({ gh, repo, sha, pr }) {
  const { failures, pending } = collect({ gh, repo, sha });
  const existing = findComment({ gh, repo, pr });
  if (failures.length > 0) {
    return upsert({ gh, repo, pr, body: buildComment({ sha, failures }), existing });
  }
  // Nothing failed. Only say "green" once nothing is still running, and only if
  // there is a failure comment to correct; never open a comment to say all is well.
  if (!pending && existing) {
    return upsert({ gh, repo, pr, body: buildResolved({ sha }), existing });
  }
  return "nothing to do";
}

function realGh(args) {
  return execFileSync("gh", args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, env: process.env });
}

function main() {
  const repo = process.env.GITHUB_REPOSITORY ?? "";
  const sha = process.env.HEAD_SHA ?? "";
  const pr = process.env.PR_NUMBER ?? "";
  // These come from the event payload, but they are interpolated into API paths,
  // so refuse anything that is not exactly the shape expected.
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo) || !/^[0-9a-f]{40}$/.test(sha) || !/^[1-9]\d*$/.test(pr)) {
    console.error("ci-feedback: GITHUB_REPOSITORY, HEAD_SHA and PR_NUMBER are required and must be well formed");
    process.exit(1);
  }
  console.log(`ci-feedback: ${run({ gh: realGh, repo, sha, pr })}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) main();
