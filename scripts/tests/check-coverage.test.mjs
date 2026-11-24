import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import { evaluate, main, renderRustCrates, renderTable, rustCrateRows } from "../check-coverage.mjs";

const thresholds = { thresholds: { web: { statements: 60, branches: 60, functions: 50, lines: 60 } } };
const rustThresholds = (lines) => ({ thresholds: { rust: { lines } } });
const webSummary = (n) => ({ total: Object.fromEntries(["statements", "branches", "functions", "lines"].map((k) => [k, { pct: n }])) });

describe(".coverage-thresholds.json", () => {
  const committed = JSON.parse(readFileSync(".coverage-thresholds.json", "utf8"));

  it("has a rationale and thresholds for the web component", () => {
    assert.ok(committed.rationale.length > 50);
    assert.deepEqual(Object.keys(committed.thresholds.web).sort(), ["branches", "functions", "lines", "statements"]);
  });

  it("gates the rust workspace on line coverage", () => {
    assert.deepEqual(Object.keys(committed.thresholds.rust), ["lines"]);
    assert.ok(committed.thresholds.rust.lines > 0 && committed.thresholds.rust.lines <= 100);
  });
});

describe("evaluate", () => {
  it("passes when every web metric meets its threshold", () => {
    assert.ok(evaluate("web", thresholds, webSummary(70)).every((r) => r.ok));
  });

  it("fails the metric that is below its threshold", () => {
    const rows = evaluate("web", thresholds, webSummary(55));
    assert.equal(rows.find((r) => r.metric === "functions").ok, true);
    assert.equal(rows.find((r) => r.metric === "lines").ok, false);
  });

  it("throws on missing thresholds or a malformed report instead of passing", () => {
    assert.throws(() => evaluate("rust", thresholds, {}), /no thresholds/);
    assert.throws(() => evaluate("web", thresholds, {}), /no total/);
  });

  for (const bad of [
    ["null", null],
    ["numeric string", "60"],
    ["zero", 0],
    ["negative", -5],
    ["greater than 100", 150],
    ["non-finite number", Number.POSITIVE_INFINITY],
  ]) {
    it(`rejects a ${bad[0]} web threshold instead of comparing it`, () => {
      for (const metric of ["statements", "branches", "functions", "lines"]) {
        const broken = { thresholds: { web: { statements: 88, branches: 83, functions: 86, lines: 89, [metric]: bad[1] } } };
        assert.throws(() => evaluate("web", broken, webSummary(70)), new RegExp(`web threshold ${metric}`));
      }
    });
  }

  it("still accepts the current committed thresholds", () => {
    assert.ok(evaluate("web", thresholds, webSummary(70)).every((r) => r.ok));
  });
});

// A trimmed cargo-llvm-cov --json export: data[0].totals and one file per source file, each with a line summary.
const rustFile = (filename, covered, count) => ({
  filename,
  summary: { lines: { count, covered, percent: (covered * 100) / count } },
});
const rustSummary = (percent, crates = {}) => ({
  data: [
    {
      totals: { lines: { count: 1000, covered: percent * 10, percent } },
      files: Object.entries(crates).flatMap(([dir, [covered, count]]) => [rustFile(`/h/r/${dir}/src/lib.rs`, covered, count)]),
    },
  ],
});

describe("evaluate (rust)", () => {
  it("passes when workspace line coverage meets its floor", () => {
    const rows = evaluate("rust", rustThresholds(85), rustSummary(87.4));
    assert.equal(rows.length, 1);
    assert.equal(rows[0].metric, "lines");
    assert.ok(rows[0].ok);
  });

  it("fails when workspace line coverage is below its floor", () => {
    const rows = evaluate("rust", rustThresholds(85), rustSummary(84.9));
    assert.equal(rows[0].ok, false);
  });

  it("reports the percentage rounded to two decimals but compares the measured value", () => {
    const rows = evaluate("rust", rustThresholds(85), rustSummary(87.39957));
    assert.equal(rows[0].actual, 87.4);
    assert.ok(rows[0].ok);
  });

  it("throws on missing thresholds or a malformed report instead of passing", () => {
    assert.throws(() => evaluate("rust", thresholds, rustSummary(87.4)), /no thresholds/);
    assert.throws(() => evaluate("rust", rustThresholds(85), {}), /no data\[0\]/);
    assert.throws(() => evaluate("rust", rustThresholds(85), { data: [] }), /no data\[0\]/);
    assert.throws(() => evaluate("rust", rustThresholds(85), { data: [{ totals: {} }] }), /totals\.lines\.percent/);
    assert.throws(() => evaluate("rust", rustThresholds(85), { data: [{ totals: { lines: { percent: "87" } } }] }), /totals\.lines\.percent/);
  });

  for (const bad of [null, "85", 0, -5, 150, Number.POSITIVE_INFINITY]) {
    it(`rejects a ${typeof bad === "string" ? "numeric string" : bad === null ? "null" : String(bad)} rust threshold instead of comparing it`, () => {
      assert.throws(() => evaluate("rust", { thresholds: { rust: { lines: bad } } }, rustSummary(87.4)), /rust threshold lines/);
    });
  }
});

describe("rustCrateRows", () => {
  const report = rustSummary(87.4, { "crates/hairspring-core": [1300, 1400], "crates/hairspring-check": [800, 900], "sdks/rust": [2700, 3000], "web/src": [1, 2] });

  it("sums line coverage per crate, sorted by crate path, and buckets what is outside one", () => {
    assert.deepEqual(rustCrateRows(report), [
      { crate: "crates/hairspring-check", covered: 800, lines: 900, pct: 88.89 },
      { crate: "crates/hairspring-core", covered: 1300, lines: 1400, pct: 92.86 },
      { crate: "other", covered: 1, lines: 2, pct: 50 },
      { crate: "sdks/rust", covered: 2700, lines: 3000, pct: 90 },
    ]);
  });

  it("throws on a malformed report", () => {
    assert.throws(() => rustCrateRows({}), /no data\[0\]/);
  });

  it("renders a per-crate table with a percentage column", () => {
    const text = renderRustCrates(rustCrateRows(report));
    assert.match(text, /\| crates\/hairspring-core \| 1400 \| 1300 \| 92\.86% \|/);
    assert.match(text, /\| sdks\/rust \|/);
  });
});

describe("main", () => {
  const files = {
    t: JSON.stringify(thresholds),
    ok: JSON.stringify(webSummary(70)),
    low: JSON.stringify(webSummary(10)),
    rust: JSON.stringify(rustThresholds(85)),
    rustOk: JSON.stringify(rustSummary(87.4, { "crates/hairspring-core": [1300, 1400] })),
    rustLow: JSON.stringify(rustSummary(84.9)),
  };
  const reader = (p) => files[p];
  const run = (summary, component = "web", tf = "t") => {
    const original = process.stdout.write;
    let out = "";
    process.stdout.write = (s) => {
      out += s;
      return true;
    };
    try {
      const code = main(["--component", component, "--summary", summary, "--thresholds", tf], reader);
      return { code, out };
    } finally {
      process.stdout.write = original;
    }
  };

  it("exits 0 above the thresholds and 1 below them", () => {
    assert.equal(run("ok").code, 0);
    assert.equal(run("low").code, 1);
  });

  it("renders a table with a status column", () => {
    assert.ok(renderTable("web", evaluate("web", thresholds, webSummary(10))).includes("FAIL"));
  });

  it("exits 0 above the rust floor and 1 below it", () => {
    assert.equal(run("rustOk", "rust", "rust").code, 0);
    assert.equal(run("rustLow", "rust", "rust").code, 1);
  });

  it("writes the per-crate line table into the summary next to the metric table", () => {
    const { out } = run("rustOk", "rust", "rust");
    assert.match(out, /## Coverage: rust/);
    assert.match(out, /## Rust coverage per crate/);
    assert.match(out, /\| crates\/hairspring-core \| 1400 \| 1300 \| 92\.86% \|/);
  });
});

