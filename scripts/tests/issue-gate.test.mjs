import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import { HEADING_PATTERN, TASK_PATTERN, hasAcceptanceCriteria } from "../issue-gate.mjs";

const WORKFLOW_PATH = ".github/workflows/issue-gate.yml";

describe("hasAcceptanceCriteria", () => {
  it("accepts an Acceptance criteria heading in any case and level", () => {
    assert.equal(hasAcceptanceCriteria("## Acceptance criteria\nIt works."), true);
    assert.equal(hasAcceptanceCriteria("### ACCEPTANCE CRITERIA"), true);
    assert.equal(hasAcceptanceCriteria("intro\n\n# acceptance Criteria\n"), true);
  });

  it("accepts a markdown task-list item, checked or not", () => {
    assert.equal(hasAcceptanceCriteria("Done when:\n- [ ] the test passes"), true);
    assert.equal(hasAcceptanceCriteria("- [x] already verified"), true);
    assert.equal(hasAcceptanceCriteria("  * [X] indented star bullet"), true);
  });

  it("rejects a body with neither", () => {
    assert.equal(hasAcceptanceCriteria("Please make it faster."), false);
    assert.equal(hasAcceptanceCriteria("- plain bullet\n- another"), false);
    assert.equal(hasAcceptanceCriteria("mentions acceptance criteria in prose"), false);
    assert.equal(hasAcceptanceCriteria("- [] not a task item"), false);
  });

  it("rejects empty, whitespace-only and missing bodies", () => {
    assert.equal(hasAcceptanceCriteria(""), false);
    assert.equal(hasAcceptanceCriteria("   \n"), false);
    assert.equal(hasAcceptanceCriteria(null), false);
    assert.equal(hasAcceptanceCriteria(undefined), false);
  });
});

describe("issue-gate workflow", () => {
  const text = readFileSync(WORKFLOW_PATH, "utf8");
  const inlined = (name) => new RegExp(`const ${name} = (.+);`).exec(text)?.[1];

  it("inlines exactly the regular expressions the helper exports", () => {
    assert.equal(inlined("HEADING"), String(HEADING_PATTERN));
    assert.equal(inlined("TASK"), String(TASK_PATTERN));
  });

  it("reads the issue body from the environment, never from an expression in a script", () => {
    assert.match(text, /ISSUE_BODY: \$\{\{ github\.event\.issue\.body \}\}/);
    const scriptLines = text.split("\n").filter((line) => !/^\s+[A-Z_]+: \$\{\{/.test(line));
    assert.equal(scriptLines.some((line) => line.includes("github.event.issue.body")), false);
  });

  it("is least-privilege: no workflow permissions, issues write on the job, no checkout or third-party action", () => {
    assert.match(text, /^permissions: \{\}$/m);
    assert.match(text, /^ {4}permissions:\n {6}issues: write$/m);
    assert.equal(/uses:/.test(text), false);
    assert.match(text, /timeout-minutes: 5/);
  });
});
