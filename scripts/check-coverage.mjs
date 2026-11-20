#!/usr/bin/env node
// Coverage ratchet check. Compares measured coverage with .coverage-thresholds.json and exits
// non-zero when any metric is below its threshold. Thresholds only go up: raise them as coverage
// grows, never lower them.
//
// Usage:
//   node scripts/check-coverage.mjs --component web --summary web/coverage/coverage-summary.json
// `web` reads a vitest json-summary (statements, branches, functions, lines). The Rust workspace is
// not gated: see the rationale in .coverage-thresholds.json.

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const METRICS = ["statements", "branches", "functions", "lines"];

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
      return { metric, actual, threshold: wanted[metric], ok: actual >= wanted[metric] };
    });
  }
  throw new Error(`unknown component "${component}"`);
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
  const rows = evaluate(component, JSON.parse(readFile(thresholdsPath, "utf8")), JSON.parse(readFile(summary, "utf8")));
  process.stdout.write(`${renderTable(component, rows)}\n`);
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
