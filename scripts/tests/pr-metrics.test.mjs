import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { percentile, renderMarkdown, summarise } from "../pr-metrics.mjs";

const NOW = new Date("2026-10-11T00:00:00Z");
const pr = (created, closed, merged) => ({ created_at: created, closed_at: closed, merged_at: merged });

describe("percentile", () => {
  it("returns null for no data and the nearest rank otherwise", () => {
    assert.equal(percentile([], 50), null);
    assert.equal(percentile([1, 2, 3, 4], 50), 2);
    assert.equal(percentile([1, 2, 3, 4], 90), 4);
  });
});

describe("summarise", () => {
  it("computes merge rate and hours to merge over the window only", () => {
    const pulls = [
      pr("2026-10-01T00:00:00Z", "2026-10-01T02:00:00Z", "2026-10-01T02:00:00Z"),
      pr("2026-10-02T00:00:00Z", "2026-10-02T10:00:00Z", "2026-10-02T10:00:00Z"),
      pr("2026-10-03T00:00:00Z", "2026-10-03T01:00:00Z", null),
      pr("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z", "2026-01-02T00:00:00Z"),
    ];

    const s = summarise(pulls, NOW, 90);

    assert.equal(s.closed, 3);
    assert.equal(s.merged, 2);
    assert.equal(s.closedUnmerged, 1);
    assert.ok(Math.abs(s.mergeRatePct - 66.67) < 0.1);
    assert.equal(s.medianHoursToMerge, 2);
    assert.equal(s.p90HoursToMerge, 10);
  });

  it("reports n/a rather than dividing by zero", () => {
    const s = summarise([], NOW);
    assert.equal(s.mergeRatePct, null);
    assert.ok(renderMarkdown(s, "o/r", NOW).includes("n/a"));
  });
});
