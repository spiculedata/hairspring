#!/usr/bin/env node
// Coverage ratchet check. Compares measured coverage with .coverage-thresholds.json and exits
// non-zero when any metric is below its threshold. Thresholds only go up: raise them as coverage
// grows, never lower them.
//
// Usage:
//   node scripts/check-coverage.mjs --component web --summary web/coverage/coverage-summary.json
//   node scripts/check-coverage.mjs --component rust --summary coverage-rust.json
// `web` reads a vitest json-summary (statements, branches, functions, lines). `rust` reads a
// cargo-llvm-cov --json export and gates the workspace's line coverage, with a per-crate breakdown
// written next to it for the job summary. See the rationale in .coverage-thresholds.json.

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const METRICS = ["statements", "branches", "functions", "lines"];

/** Validates one configured threshold. Returns the number; throws on anything a comparison would silently coerce. */
function threshold(component, metric, value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0 || value > 100) {
    throw new Error(`${component} threshold ${metric} must be a number greater than 0 and at most 100 (got ${String(value)})`);
  }
  return value;
}

/** Groups one cargo-llvm-cov file under the crate path that holds it, or "other". */
function rustCrate(filename) {
  const crate = /^(?:.*\/)?(crates\/[A-Za-z0-9_-]+|sdks\/rust|experiments\/[A-Za-z0-9_-]+)\//.exec(filename);
  return crate ? crate[1] : "other";
}

/** Reads the workspace line percentage out of a cargo-llvm-cov --json export; throws on malformed input. */
function rustLinesPercent(report) {
  const data = report?.data?.[0];
  if (!data) throw new Error("rust summary has no data[0] (is it a cargo-llvm-cov --json export?)");
  const percent = data.totals?.lines?.percent;
  if (typeof percent !== "number") throw new Error("rust summary has no data[0].totals.lines.percent");
  return percent;
}

/** Returns [{ metric, actual, threshold, ok }] for one component. Pure; throws on bad input. */
export function evaluate(component, thresholds, report) {
  const wanted = thresholds?.thresholds?.[component];
  if (!wanted) throw new Error(`no thresholds for component "${component}"`);
  if (component === "web") {
    const total = report?.total;
    if (!total) throw new Error("web summary has no total (is it a vitest json-summary?)");
    return METRICS.map((metric) => {
      const actual = total[metric]?.pct;
      if (typeof actual !== "number") throw new Error(`web summary has no ${metric}.pct`);
      const limit = threshold(component, metric, wanted[metric]);
      return { metric, actual, threshold: limit, ok: actual >= limit };
    });
  }
  if (component === "rust") {
    const measured = rustLinesPercent(report);
    const actual = Math.round(measured * 100) / 100;
    const limit = threshold(component, "lines", wanted["lines"]);
    return [{ metric: "lines", actual, threshold: limit, ok: measured >= limit }];
  }
  throw new Error(`unknown component "${component}"`);
}

/** Line coverage per crate, from a cargo-llvm-cov --json export. Returns [{ crate, lines, covered, pct }], sorted by crate. */
export function rustCrateRows(report) {
  const data = report?.data?.[0];
  if (!data) throw new Error("rust summary has no data[0] (is it a cargo-llvm-cov --json export?)");
  const byCrate = new Map();
  for (const file of data.files ?? []) {
    const crate = rustCrate(file.filename ?? "");
    const soFar = byCrate.get(crate) ?? [0, 0];
    soFar[0] += file.summary?.lines?.covered ?? 0;
    soFar[1] += file.summary?.lines?.count ?? 0;
    byCrate.set(crate, soFar);
  }
  return [...byCrate]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([crate, [covered, lines]]) => ({
      crate,
      lines,
      covered,
      pct: lines > 0 ? Math.round((covered * 100 * 100) / lines) / 100 : 0,
    }));
}

export function renderRustCrates(rows) {
  const lines = ["## Rust coverage per crate", "", "| Crate | Lines | Covered | Percent |", "| --- | --- | --- | --- |"];
  for (const r of rows) lines.push(`| ${r.crate} | ${r.lines} | ${r.covered} | ${r.pct}% |`);
  return lines.join("\n");
}

export function renderTable(component, rows) {
  const lines = [`## Coverage: ${component}`, "", "| Metric | Actual | Threshold | Status |", "| --- | --- | --- | --- |"];
  for (const r of rows) lines.push(`| ${r.metric} | ${r.actual}% | ${r.threshold}% | ${r.ok ? "pass" : "FAIL"} |`);
  return lines.join("\n");
}

export function main(argv, readFile = readFileSync) {
  const arg = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const component = arg("--component");
  const summary = arg("--summary");
  const thresholdsPath = arg("--thresholds") ?? ".coverage-thresholds.json";
  if (!component || !summary) throw new Error("--component and --summary are required");
  const report = JSON.parse(readFile(summary, "utf8"));
  const rows = evaluate(component, JSON.parse(readFile(thresholdsPath, "utf8")), report);
  process.stdout.write(`${renderTable(component, rows)}\n`);
  if (component === "rust") process.stdout.write(`${renderRustCrates(rustCrateRows(report))}\n`);
  return rows.every((r) => r.ok) ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exit(main(process.argv.slice(2)));
  } catch (err) {
    process.stderr.write(`check-coverage: ${err.message}\n`);
    process.exit(2);
  }
}
