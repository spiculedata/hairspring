#!/usr/bin/env node
// Quality report: one page that answers "how healthy is this codebase right now?"
//
// The workflow (.github/workflows/quality-report.yml) runs the real tools and saves their
// output into a directory; this script only reads those files and renders Markdown, so every
// number here comes from a tool, never from the script. A missing input is reported as
// "not available" rather than guessed or shown as zero.
//
// Usage: node scripts/quality-report.mjs --in quality --out quality/quality-report.md
//   Rust workspace: rust-test.txt (`cargo test --workspace --no-fail-fast` output),
//     rust-clippy.txt (`cargo clippy --workspace --all-targets --message-format=short` output),
//     rust-audit.json (`cargo audit --json`)
//   web UI: web-vitest.json (vitest --reporter=json), web-audit-prod.json, web-audit-all.json
//     (npm audit --json), web-coverage-summary.json (vitest json-summary)
// Rust coverage is not part of this report: no coverage tool is wired for the Rust workspace yet
// (see .coverage-thresholds.json).

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

/** Sums the `test result:` lines cargo prints for every test binary and doc-test run. */
export function summariseCargoTest(text) {
  if (typeof text !== "string") return null;
  const results = [...text.matchAll(/^test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored/gm)];
  if (results.length === 0) return null;
  const sum = (index) => results.reduce((total, match) => total + Number(match[index]), 0);
  return { binaries: results.length, passed: sum(1), failed: sum(2), ignored: sum(3) };
}

/** Counts rustc and clippy diagnostics in `--message-format=short` output (`path:line:col: warning: ...`). */
export function summariseClippy(text) {
  if (typeof text !== "string") return null;
  const count = (level) => [...text.matchAll(new RegExp(`^\\S+:\\d+:\\d+: ${level}\\b`, "gm"))].length;
  return { warnings: count("warning"), errors: count("error") };
}

/** `cargo audit --json`: vulnerabilities plus the unmaintained, unsound and yanked warnings. */
export function summariseCargoAudit(report) {
  const count = report?.vulnerabilities?.count;
  if (typeof count !== "number") return null;
  const warnings = report.warnings ?? {};
  const size = (kind) => (Array.isArray(warnings[kind]) ? warnings[kind].length : 0);
  return { vulnerabilities: count, unmaintained: size("unmaintained"), unsound: size("unsound"), yanked: size("yanked") };
}

export function summariseNpmAudit(report) {
  const counts = report?.metadata?.vulnerabilities;
  if (!counts) return null;
  return {
    critical: counts.critical ?? 0,
    high: counts.high ?? 0,
    moderate: counts.moderate ?? 0,
    low: counts.low ?? 0,
  };
}

export function summariseVitest(report) {
  if (!report || typeof report.numTotalTests !== "number") return null;
  return {
    suites: report.numTotalTestSuites ?? 0,
    tests: report.numTotalTests,
    passed: report.numPassedTests ?? 0,
    failed: report.numFailedTests ?? 0,
  };
}

export function summariseVitestCoverage(summary) {
  const total = summary?.total;
  if (!total) return null;
  const pct = (key) => (typeof total[key]?.pct === "number" ? total[key].pct : null);
  return { lines: pct("lines"), statements: pct("statements"), functions: pct("functions"), branches: pct("branches") };
}

const na = "not available";
const pct = (value) => (value === null || value === undefined ? na : `${value}%`);
const sev = (a) => `${a.critical} critical, ${a.high} high, ${a.moderate} moderate, ${a.low} low`;

export function renderReport(data, repo, now) {
  const { rust, web } = data;
  const lines = [];
  lines.push(`# Quality report: ${repo}`);
  lines.push("");
  lines.push(`Generated ${now.toISOString()} from the tools' own output.`);
  lines.push("");
  lines.push("| Signal | Result |");
  lines.push("| --- | --- |");
  lines.push(
    `| Rust tests (cargo test) | ${rust.tests ? `${rust.tests.passed} passed, ${rust.tests.failed} failed, ${rust.tests.ignored} ignored, across ${rust.tests.binaries} test binaries` : na} |`,
  );
  lines.push(`| Rust lint (clippy) | ${rust.clippy ? `${rust.clippy.warnings} warnings, ${rust.clippy.errors} errors` : na} |`);
  lines.push(
    `| Rust audit (cargo audit) | ${rust.audit ? `${rust.audit.vulnerabilities} vulnerabilities, ${rust.audit.unmaintained} unmaintained, ${rust.audit.unsound} unsound, ${rust.audit.yanked} yanked` : na} |`,
  );
  lines.push("| Rust coverage | not measured (no coverage tool is wired for the Rust workspace) |");
  lines.push(
    `| Web tests (Vitest) | ${web.tests ? `${web.tests.passed}/${web.tests.tests} passed, ${web.tests.failed} failed, ${web.tests.suites} suites` : na} |`,
  );
  lines.push(`| Web audit (production) | ${web.auditProd ? sev(web.auditProd) : na} |`);
  lines.push(`| Web audit (all, incl. dev) | ${web.auditAll ? sev(web.auditAll) : na} |`);
  lines.push(
    `| Web coverage | ${web.coverage ? `lines ${pct(web.coverage.lines)}, statements ${pct(web.coverage.statements)}, functions ${pct(web.coverage.functions)}, branches ${pct(web.coverage.branches)}` : na} |`,
  );
  lines.push("");
  const gate = [];
  if (rust.tests && rust.tests.failed > 0) gate.push(`${rust.tests.failed} failing Rust tests`);
  if (rust.clippy && rust.clippy.warnings + rust.clippy.errors > 0) gate.push(`${rust.clippy.warnings + rust.clippy.errors} clippy diagnostics`);
  if (rust.audit && rust.audit.vulnerabilities > 0) gate.push(`${rust.audit.vulnerabilities} RustSec vulnerabilities`);
  if (web.tests && web.tests.failed > 0) gate.push(`${web.tests.failed} failing web tests`);
  if (web.auditProd && (web.auditProd.critical > 0 || web.auditProd.high > 0)) gate.push("web production audit has HIGH or CRITICAL advisories");
  const missing = [
    ["Rust tests", rust.tests],
    ["Rust lint", rust.clippy],
    ["Rust audit", rust.audit],
    ["web tests", web.tests],
    ["web production audit", web.auditProd],
    ["web full audit", web.auditAll],
    ["web coverage", web.coverage],
  ]
    .filter(([, value]) => value === null)
    .map(([name]) => name);
  if (gate.length) lines.push(`**Needs attention:** ${gate.join("; ")}.`);
  if (missing.length) lines.push(`**Report incomplete:** unavailable signals: ${missing.join(", ")}.`);
  if (!gate.length && !missing.length) lines.push("**Gate signals:** nothing blocking in what was measured.");
  lines.push("");
  lines.push("`ci` is the real gate; this page is the wider picture. See docs/quality.md for how each number is produced.");
  lines.push("");
  return lines.join("\n");
}

function readText(path) {
  if (!existsSync(path)) return null;
  try {
    return readFileSync(path, "utf8");
  } catch {
    return null;
  }
}

function readJson(path) {
  const text = readText(path);
  if (text === null) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

export function loadInputs(dir) {
  return {
    rust: {
      tests: summariseCargoTest(readText(join(dir, "rust-test.txt"))),
      clippy: summariseClippy(readText(join(dir, "rust-clippy.txt"))),
      audit: summariseCargoAudit(readJson(join(dir, "rust-audit.json"))),
    },
    web: {
      tests: summariseVitest(readJson(join(dir, "web-vitest.json"))),
      auditProd: summariseNpmAudit(readJson(join(dir, "web-audit-prod.json"))),
      auditAll: summariseNpmAudit(readJson(join(dir, "web-audit-all.json"))),
      coverage: summariseVitestCoverage(readJson(join(dir, "web-coverage-summary.json"))),
    },
  };
}

export function main(argv) {
  const arg = (name, fallback) => {
    const i = argv.indexOf(name);
    return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
  };
  const dir = arg("--in", "quality");
  const out = arg("--out", join(dir, "quality-report.md"));
  const repo = process.env.GITHUB_REPOSITORY || "spiculedata/hairspring";
  const markdown = renderReport(loadInputs(dir), repo, new Date());
  writeFileSync(out, markdown);
  process.stdout.write(markdown);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
