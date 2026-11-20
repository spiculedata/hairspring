#!/usr/bin/env node
// PR acceptance metric: of the pull requests closed in the last N days, how many
// merged, and how long did merging take? Computed from the GitHub API and
// rendered to markdown (docs/metrics.md by default). No opinion about whether a
// given PR was good; the point is a number to compare with last month's.
//
// Usage:
//   GITHUB_TOKEN=... GITHUB_REPOSITORY=owner/repo \
//     node scripts/pr-metrics.mjs [--window-days 90] [--out docs/metrics.md]

import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const API = "https://api.github.com";
const DEFAULT_WINDOW_DAYS = 90;
const MAX_PAGES = 10; // 1000 closed PRs is far more than this repo produces
const API_REQUEST_TIMEOUT_MS = 30_000;
const HOUR_MS = 3_600_000;

// --- pure computation --------------------------------------------------------

export function percentile(values, p) {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank))];
}

/**
 * pulls: API pull objects (state closed). Only PRs closed inside the window count.
 * Returns counts, merge rate (merged / closed) and time-to-merge in hours.
 */
export function summarise(pulls, now, windowDays = DEFAULT_WINDOW_DAYS) {
  const cutoff = now.getTime() - windowDays * 24 * HOUR_MS;
  const inWindow = pulls.filter((pr) => pr.closed_at && Date.parse(pr.closed_at) >= cutoff);
  const merged = inWindow.filter((pr) => pr.merged_at);
  const hours = merged.map((pr) => (Date.parse(pr.merged_at) - Date.parse(pr.created_at)) / HOUR_MS);
  return {
    windowDays,
    closed: inWindow.length,
    merged: merged.length,
    closedUnmerged: inWindow.length - merged.length,
    mergeRatePct: inWindow.length === 0 ? null : (merged.length / inWindow.length) * 100,
    medianHoursToMerge: percentile(hours, 50),
    p90HoursToMerge: percentile(hours, 90),
  };
}

const fmt = (n, digits = 1) => (n === null ? "n/a" : n.toFixed(digits));

export function renderMarkdown(summary, repo, now) {
  return [
    "# Pull request acceptance",
    "",
    `Generated ${now.toISOString().slice(0, 10)} by scripts/pr-metrics.mjs for ${repo}. Do not edit by hand.`,
    "",
    `Window: pull requests closed in the last ${summary.windowDays} days.`,
    "",
    "| Metric | Value |",
    "| --- | --- |",
    `| Closed | ${summary.closed} |`,
    `| Merged | ${summary.merged} |`,
    `| Closed without merging | ${summary.closedUnmerged} |`,
    `| Merge rate | ${fmt(summary.mergeRatePct)}% |`,
    `| Median time to merge | ${fmt(summary.medianHoursToMerge)} h |`,
    `| p90 time to merge | ${fmt(summary.p90HoursToMerge)} h |`,
    "",
    "Time to merge runs from the PR being opened to it being merged. Merge rate is merged divided by closed, so a PR that is still open is not counted either way.",
    "",
  ].join("\n");
}

// --- GitHub API ---------------------------------------------------------------

export async function fetchClosedPulls(repo, token, cutoffMs, request = fetch, requestTimeoutMs = API_REQUEST_TIMEOUT_MS) {
  const pulls = [];
  for (let page = 1; page <= MAX_PAGES; page += 1) {
    const res = await request(
      `${API}/repos/${repo}/pulls?state=closed&sort=updated&direction=desc&per_page=100&page=${page}`,
      {
        signal: AbortSignal.timeout(requestTimeoutMs),
        headers: {
          accept: "application/vnd.github+json",
          authorization: `Bearer ${token}`,
          "x-github-api-version": "2022-11-28",
        },
      },
    );
    if (!res.ok) throw new Error(`GitHub API ${res.status} listing pull requests (page ${page})`);
    const batch = await res.json();
    pulls.push(...batch);
    // Sorted by updated desc: once a whole page is older than the window, stop.
    const oldest = batch.at(-1);
    if (batch.length < 100 || (oldest && Date.parse(oldest.updated_at) < cutoffMs)) break;
  }
  return pulls;
}

function parseArgs(argv) {
  const args = { windowDays: DEFAULT_WINDOW_DAYS, out: "docs/metrics.md" };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--window-days") args.windowDays = Number(argv[++i]);
    else if (a === "--out") args.out = argv[++i];
    else throw new Error(`unknown argument ${a}`);
  }
  if (!Number.isInteger(args.windowDays) || args.windowDays <= 0) throw new Error("--window-days must be a positive integer");
  return args;
}

export async function main(argv) {
  const args = parseArgs(argv);
  const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN;
  const repo = process.env.GITHUB_REPOSITORY;
  if (!token || !repo) throw new Error("GITHUB_TOKEN (or GH_TOKEN) and GITHUB_REPOSITORY are required");
  const now = new Date();
  const cutoff = now.getTime() - args.windowDays * 24 * HOUR_MS;
  const pulls = await fetchClosedPulls(repo, token, cutoff);
  const markdown = renderMarkdown(summarise(pulls, now, args.windowDays), repo, now);
  writeFileSync(args.out, markdown);
  process.stdout.write(markdown);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (err) => {
      process.stderr.write(`pr-metrics: ${err.message}\n`);
      process.exit(1);
    },
  );
}
