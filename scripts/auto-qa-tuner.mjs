#!/usr/bin/env node
// Auto-QA self-tuning for Hairspring.
//
// Reads recent `ci` workflow runs on main from the GitHub API, works out what a
// sensible timeout / retry allowance / pass-rate floor would be, and rewrites the
// `value` of each parameter in .github/auto-qa-tuning.json. The workflow
// (.github/workflows/auto-qa.yml) opens a pull request with the result. This
// script never pushes anything and never touches main.
//
// Honest scope: the values are advisory. Nothing in ci.yml reads them. The point
// is a reviewed, bounded, versioned record of what the CI history says the gates
// could be, which a human can then wire in.
//
// Safety properties, all enforced by scripts/tests/auto-qa-tuning.test.mjs:
//   - only `value` and `history` are ever written; baseline, min, max, maxStep
//     are human-owned, so the tuner cannot widen its own bounds;
//   - every move is at most maxStep and ends inside [min, max], so a gate can
//     never be loosened past the documented limit however bad the data is;
//   - with fewer than minObservations runs it changes nothing.
//
// Usage:
//   node scripts/auto-qa-tuner.mjs --check
//   GITHUB_TOKEN=... GITHUB_REPOSITORY=owner/repo \
//     node scripts/auto-qa-tuner.mjs --from-ci --write --out tuned.json
//   node scripts/auto-qa-tuner.mjs --observations obs.json   (offline, prints result)

import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const CONFIG_PATH = ".github/auto-qa-tuning.json";
const API = "https://api.github.com";
const LOOSEN_DIRECTIONS = new Set(["up", "down"]);

// --- pure maths ------------------------------------------------------------

export function percentile(values, p) {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank))];
}

const clamp = (value, lo, hi) => Math.min(hi, Math.max(lo, value));

/** Move `current` toward `target` by at most maxStep, staying inside [min, max]. */
export function stepToward(param, target) {
  const delta = clamp(target - param.value, -param.maxStep, param.maxStep);
  return clamp(Math.round(param.value + delta), param.min, param.max);
}

/**
 * Summarise run records into the observations the targets are computed from.
 * A record is { conclusion, runAttempt, jobSeconds, testSeconds } where the
 * seconds are null when the job or step was not found.
 */
export function summarise(records) {
  const done = records.filter((r) => r.conclusion === "success" || r.conclusion === "failure");
  const successes = done.filter((r) => r.conclusion === "success");
  const recovered = successes.filter((r) => r.runAttempt > 1);
  return {
    runs: done.length,
    passRatePct: done.length === 0 ? null : (successes.length / done.length) * 100,
    flakyRecoveryRate: successes.length === 0 ? 0 : recovered.length / successes.length,
    p95JobSeconds: percentile(successes.map((r) => r.jobSeconds).filter((v) => v !== null), 95),
    p95TestSeconds: percentile(successes.map((r) => r.testSeconds).filter((v) => v !== null), 95),
  };
}

/** The value each parameter would ideally take, or null if there is no data for it. */
export function targets(config, obs) {
  const threshold = config.controller.flakyRecoveryRateThreshold;
  return {
    ciTimeoutMinutes: obs.p95JobSeconds === null ? null : Math.ceil((obs.p95JobSeconds * 2) / 60),
    testStepBudgetSeconds: obs.p95TestSeconds === null ? null : Math.ceil(obs.p95TestSeconds * 3),
    flakeRetries: obs.flakyRecoveryRate >= threshold ? 1 : 0,
    ciPassRateFloorPct: obs.passRatePct === null ? null : Math.floor(obs.passRatePct) - 5,
  };
}

/** Returns a new config; the input is not mutated. */
export function tune(config, obs, now = new Date()) {
  if (obs.runs < config.controller.minObservations) return config;
  const wanted = targets(config, obs);
  const changes = [];
  const parameters = config.parameters.map((param) => {
    const target = wanted[param.id];
    if (target === null || target === undefined) return param;
    const next = stepToward(param, target);
    if (next === param.value) return param;
    changes.push({ id: param.id, from: param.value, to: next, target });
    return { ...param, value: next };
  });
  if (changes.length === 0) return config;
  const entry = { at: now.toISOString(), runs: obs.runs, changes };
  const history = [entry, ...config.history].slice(0, config.controller.historyLimit);
  return { ...config, parameters, history };
}

/** Invariant violations as strings; empty means the config is sound. */
export function validate(config) {
  const problems = [];
  if (!Array.isArray(config.parameters) || config.parameters.length === 0) {
    return ["parameters must be a non-empty array"];
  }
  const seen = new Set();
  for (const p of config.parameters) {
    const at = `parameter ${p.id}`;
    if (seen.has(p.id)) problems.push(`${at}: duplicate id`);
    seen.add(p.id);
    for (const field of ["value", "baseline", "min", "max", "maxStep"]) {
      if (!Number.isFinite(p[field])) problems.push(`${at}: ${field} must be a number`);
    }
    if (!LOOSEN_DIRECTIONS.has(p.loosen)) problems.push(`${at}: loosen must be "up" or "down"`);
    if (p.min > p.max) problems.push(`${at}: min is above max`);
    if (p.value < p.min || p.value > p.max) problems.push(`${at}: value ${p.value} is outside [${p.min}, ${p.max}]`);
    if (p.baseline < p.min || p.baseline > p.max) problems.push(`${at}: baseline is outside [${p.min}, ${p.max}]`);
    if (!(p.maxStep > 0)) problems.push(`${at}: maxStep must be positive`);
  }
  if (!(config.controller?.minObservations > 0)) problems.push("controller.minObservations must be positive");
  return problems;
}

// --- GitHub API (the only part that needs a token) ---------------------------

async function gh(path, token) {
  const res = await fetch(`${API}${path}`, {
    headers: {
      accept: "application/vnd.github+json",
      authorization: `Bearer ${token}`,
      "x-github-api-version": "2022-11-28",
    },
  });
  if (!res.ok) throw new Error(`GitHub API ${res.status} for ${path}`);
  return res.json();
}

const seconds = (start, end) =>
  start && end ? Math.max(0, (Date.parse(end) - Date.parse(start)) / 1000) : null;

export async function collectRecords(config, repo, token, request = gh) {
  const { workflowFile, branch, jobName, testStepName } = config.source;
  const list = await request(
    `/repos/${repo}/actions/workflows/${workflowFile}/runs?branch=${branch}&status=completed&per_page=${config.controller.windowRuns}`,
    token,
  );
  const records = [];
  for (const run of list.workflow_runs ?? []) {
    const jobs = await request(`/repos/${repo}/actions/runs/${run.id}/jobs?filter=latest&per_page=100`, token);
    const job = (jobs.jobs ?? []).find((j) => j.name === jobName);
    const step = job?.steps?.find((s) => s.name === testStepName);
    records.push({
      conclusion: run.conclusion,
      runAttempt: run.run_attempt ?? 1,
      jobSeconds: job ? seconds(job.started_at, job.completed_at) : null,
      testSeconds: step ? seconds(step.started_at, step.completed_at) : null,
    });
  }
  return records;
}

// --- CLI -------------------------------------------------------------------

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--check") args.check = true;
    else if (a === "--from-ci") args.fromCi = true;
    else if (a === "--write") args.write = true;
    else if (a === "--out") args.out = argv[++i];
    else if (a === "--observations") args.observations = argv[++i];
    else throw new Error(`unknown argument ${a}`);
  }
  return args;
}

export async function main(argv) {
  const args = parseArgs(argv);
  const config = JSON.parse(readFileSync(CONFIG_PATH, "utf8"));
  const problems = validate(config);
  if (problems.length > 0) throw new Error(`${CONFIG_PATH} is invalid:\n- ${problems.join("\n- ")}`);
  if (args.check) {
    process.stdout.write(`${CONFIG_PATH}: ${config.parameters.length} parameters, all inside their bounds\n`);
    return 0;
  }

  let obs;
  if (args.observations) {
    obs = JSON.parse(readFileSync(args.observations, "utf8"));
  } else if (args.fromCi) {
    const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN;
    const repo = process.env.GITHUB_REPOSITORY;
    if (!token || !repo) throw new Error("--from-ci needs GITHUB_TOKEN (or GH_TOKEN) and GITHUB_REPOSITORY");
    obs = summarise(await collectRecords(config, repo, token));
  } else {
    throw new Error("nothing to do: pass --check, --from-ci or --observations");
  }

  const tuned = tune(config, obs);
  const text = `${JSON.stringify(tuned, null, 2)}\n`;
  process.stderr.write(`observed ${JSON.stringify(obs)}\n`);
  process.stderr.write(tuned === config ? "no change\n" : "retuned\n");
  if (args.write) writeFileSync(args.out ?? CONFIG_PATH, text);
  else process.stdout.write(text);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (err) => {
      process.stderr.write(`auto-qa-tuner: ${err.message}\n`);
      process.exit(1);
    },
  );
}
