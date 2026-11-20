import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import {
  loadInputs,
  renderReport,
  summariseCargoAudit,
  summariseCargoTest,
  summariseClippy,
  summariseNpmAudit,
  summariseVitest,
  summariseVitestCoverage,
} from "../quality-report.mjs";

const NOW = new Date("2026-11-20T00:00:00Z");
const empty = {
  rust: { tests: null, clippy: null, audit: null },
  web: { tests: null, auditProd: null, auditAll: null, coverage: null },
};
const clean = {
  rust: { tests: { binaries: 3, passed: 10, failed: 0, ignored: 1 }, clippy: { warnings: 0, errors: 0 }, audit: { vulnerabilities: 0, unmaintained: 0, unsound: 0, yanked: 0 } },
  web: {
    tests: { suites: 1, tests: 2, passed: 2, failed: 0 },
    auditProd: { critical: 0, high: 0, moderate: 0, low: 0 },
    auditAll: { critical: 0, high: 0, moderate: 0, low: 0 },
    coverage: { lines: 80, statements: 80, functions: 80, branches: 80 },
  },
};

describe("summarise inputs", () => {
  it("sums every `test result:` line of cargo test and returns null without any", () => {
    const text = [
      "running 3 tests",
      "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
      "test result: FAILED. 4 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s",
    ].join("\n");
    assert.deepEqual(summariseCargoTest(text), { binaries: 2, passed: 7, failed: 2, ignored: 1 });
    assert.equal(summariseCargoTest("error: could not compile"), null);
    assert.equal(summariseCargoTest(null), null);
  });

  it("counts clippy warnings and errors in short message format", () => {
    const text = [
      "crates/a/src/lib.rs:10:5: warning: unused variable: `x`",
      "crates/a/src/lib.rs:11:5: error: this could be a `let`",
      "warning: `a` (lib) generated 1 warning",
    ].join("\n");
    assert.deepEqual(summariseClippy(text), { warnings: 1, errors: 1 });
    assert.deepEqual(summariseClippy(""), { warnings: 0, errors: 0 });
    assert.equal(summariseClippy(undefined), null);
  });

  it("reads cargo audit vulnerabilities and warnings, and returns null without a count", () => {
    const report = { vulnerabilities: { count: 1, list: [{}] }, warnings: { unmaintained: [{}, {}], yanked: [{}] } };
    assert.deepEqual(summariseCargoAudit(report), { vulnerabilities: 1, unmaintained: 2, unsound: 0, yanked: 1 });
    assert.equal(summariseCargoAudit({}), null);
  });

  it("reads npm audit severities and returns null without metadata", () => {
    assert.deepEqual(summariseNpmAudit({ metadata: { vulnerabilities: { critical: 0, high: 2, moderate: 3, low: 1 } } }), {
      critical: 0,
      high: 2,
      moderate: 3,
      low: 1,
    });
    assert.equal(summariseNpmAudit({}), null);
  });

  it("reads vitest json totals", () => {
    assert.deepEqual(summariseVitest({ numTotalTestSuites: 4, numTotalTests: 10, numPassedTests: 9, numFailedTests: 1 }), {
      suites: 4,
      tests: 10,
      passed: 9,
      failed: 1,
    });
    assert.equal(summariseVitest(null), null);
  });

  it("reads coverage percentages", () => {
    assert.deepEqual(summariseVitestCoverage({ total: { lines: { pct: 81.5 }, branches: { pct: 70 } } }), {
      lines: 81.5,
      statements: null,
      functions: null,
      branches: 70,
    });
    assert.equal(summariseVitestCoverage({}), null);
  });
});

describe("loadInputs", () => {
  it("returns null for every missing or unparsable file", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "quality-"));
    writeFileSync(path.join(dir, "web-vitest.json"), "{not json");
    assert.deepEqual(loadInputs(dir), empty);
  });

  it("reads the files the workflow writes", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "quality-"));
    writeFileSync(path.join(dir, "rust-test.txt"), "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n");
    writeFileSync(path.join(dir, "web-coverage-summary.json"), JSON.stringify({ total: { lines: { pct: 90 } } }));
    const inputs = loadInputs(dir);
    assert.equal(inputs.rust.tests.passed, 5);
    assert.equal(inputs.web.coverage.lines, 90);
  });
});

describe("renderReport", () => {
  it('says "not available" for missing inputs instead of showing zeros', () => {
    const md = renderReport(empty, "o/r", NOW);
    assert.ok(md.includes("not available"));
    assert.ok(md.includes("**Report incomplete:**"));
    assert.ok(md.includes("Rust tests, Rust lint, Rust audit, web tests"));
    assert.doesNotMatch(md, /nothing blocking in what was measured/);
    assert.doesNotMatch(md, /0 failed/);
  });

  it("states that Rust coverage is not measured rather than leaving it out", () => {
    assert.ok(renderReport(clean, "o/r", NOW).includes("| Rust coverage | not measured"));
  });

  it("keeps the clean summary when all signals are available", () => {
    const md = renderReport(clean, "o/r", NOW);
    assert.ok(md.includes("**Gate signals:** nothing blocking in what was measured."));
    assert.doesNotMatch(md, /Report incomplete/);
  });

  it("reports a missing signal alongside a measured failure", () => {
    const md = renderReport({ rust: { ...empty.rust, tests: { binaries: 1, passed: 1, failed: 1, ignored: 0 } }, web: empty.web }, "o/r", NOW);
    assert.ok(md.includes("**Needs attention:** 1 failing Rust tests."));
    assert.ok(md.includes("**Report incomplete:** unavailable signals: Rust lint"));
  });

  it("flags vulnerabilities, clippy diagnostics, production advisories and failing tests", () => {
    const md = renderReport(
      {
        rust: { tests: { binaries: 1, passed: 1, failed: 0, ignored: 0 }, clippy: { warnings: 2, errors: 1 }, audit: { vulnerabilities: 1, unmaintained: 0, unsound: 0, yanked: 0 } },
        web: { ...clean.web, auditProd: { critical: 0, high: 1, moderate: 0, low: 0 }, tests: { suites: 1, tests: 3, passed: 2, failed: 1 } },
      },
      "o/r",
      NOW,
    );
    assert.ok(md.includes("3 clippy diagnostics"));
    assert.ok(md.includes("1 RustSec vulnerabilities"));
    assert.ok(md.includes("HIGH or CRITICAL"));
    assert.ok(md.includes("1 failing web tests"));
  });
});

describe("the workflow and the docs", () => {
  it("quality-report.yml writes exactly the files loadInputs reads", () => {
    const workflow = readFileSync(".github/workflows/quality-report.yml", "utf8");
    for (const file of ["rust-test.txt", "rust-clippy.txt", "rust-audit.json", "web-vitest.json", "web-audit-prod.json", "web-audit-all.json", "web-coverage-summary.json"]) {
      assert.ok(workflow.includes(file), `${file} is not produced by the workflow`);
    }
  });
});
