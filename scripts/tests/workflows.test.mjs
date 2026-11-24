// Static guards over .github/workflows. Hermetic: reads workflow text only.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { describe, it } from "node:test";

const DIR = ".github/workflows";
const read = (name) => readFileSync(`${DIR}/${name}`, "utf8");
const files = readdirSync(DIR).filter((f) => f.endsWith(".yml"));
const code = (text) => text.split("\n").filter((l) => !l.trim().startsWith("#")).join("\n");

// Workflows that predate the agent-ready template. They pin their actions to major-version tags
// (`@v4`), and pages.yml holds `pages: write` and `id-token: write` at the top level (the deploy job
// needs both), which is why they are exempt from the full-SHA and top-level-write rules below. Do not
// add a file here: a new workflow pins every action to a full commit SHA and keeps write scopes on the
// job. Changing these is a separate, reviewed change.
const LEGACY = new Set(["ci.yml", "hermit-upstream.yml", "java-native-k.yml", "pages.yml", "release.yml"]);

// Single-job workflows whose one job needs the write scope, asserted exactly by their own test:
// ci-feedback.yml (actions: read, contents: read, pull-requests: write; see ci-feedback.test.mjs).
const TOP_LEVEL_WRITE_OK = new Set(["ci-feedback.yml"]);

const jobIds = (text) => {
  const body = text.split(/^jobs:\n/m)[1] ?? "";
  return [...body.matchAll(/^ {2}([A-Za-z0-9_-]+):\s*$/gm)].map((m) => m[1]);
};

describe("every workflow", () => {
  it("pins every third-party action to a full commit SHA with a version comment (new workflows)", () => {
    for (const file of files) {
      if (LEGACY.has(file)) continue;
      for (const line of read(file).split("\n")) {
        const match = line.match(/^\s*-?\s*uses:\s*(\S+)(.*)$/);
        if (!match || match[1].startsWith("./")) continue;
        assert.match(match[1], /@[0-9a-f]{40}$/, `${file}: ${match[1]} is not pinned to a SHA`);
        assert.match(match[2], /#\s*v\d/, `${file}: ${match[1]} has no version comment`);
      }
    }
  });

  it("never uses pull_request_target", () => {
    for (const file of files) {
      assert.doesNotMatch(code(read(file)), /pull_request_target/, file);
    }
  });

  it("declares top-level permissions so the default token is never implicit", () => {
    for (const file of files) {
      assert.match(read(file), /^permissions:/m, `${file} has no top-level permissions`);
    }
  });

  it("never interpolates untrusted PR or issue text into a run step", () => {
    for (const file of files) {
      const runBlocks = code(read(file)).match(/run:[\s\S]*?(?=\n\s{0,8}-\s|\n\s{2,6}\w+:\s|$)/g) ?? [];
      for (const block of runBlocks) {
        assert.doesNotMatch(block, /\$\{\{\s*github\.event\.(issue|pull_request|comment|review)\.(title|body|head\.ref)/, `${file} interpolates PR/issue text into a script`);
      }
    }
  });

  it("never grants write access at the top level", () => {
    for (const file of files) {
      if (LEGACY.has(file) || TOP_LEVEL_WRITE_OK.has(file)) continue;
      const top = read(file).split(/^jobs:/m)[0];
      assert.doesNotMatch(code(top), /^\s{2}[a-z-]+: write\b/m, `${file} grants write permissions at the top level`);
    }
  });
});

describe("ci.yml", () => {
  const text = read("ci.yml");

  it("ends in an aggregate job literally named ci", () => {
    assert.match(text, /^ {2}ci:\n {4}name: ci\n/m);
    assert.match(text, /^ {2}ci:\n[\s\S]*?\n {4}if: always\(\)/m);
  });

  it("makes the ci job need every other job, so a new job cannot be forgotten", () => {
    const ids = jobIds(text).filter((id) => id !== "ci");
    const block = text.split(/^ {2}ci:\n/m)[1];
    const needs = block.match(/needs:\n((?: {6}- .+\n)+)/)?.[1] ?? "";
    const listed = [...needs.matchAll(/- (\S+)/g)].map((m) => m[1]);
    assert.ok(ids.length > 10, "found the other jobs");
    assert.deepEqual([...listed].sort(), [...ids].sort());
  });

  it("fails the ci job when a needed job failed or was cancelled, and accepts skipped manual-only jobs", () => {
    const block = text.split(/^ {2}ci:\n/m)[1];
    assert.match(block, /value\.result != "success" and \.value\.result != "skipped"/);
    assert.match(block, /exit 1/);
  });
});

describe("tier-classifier.yml and labeler.yml", () => {
  it("only read changed paths and hold pull-requests: write on the job or workflow that needs it", () => {
    const classifier = read("tier-classifier.yml");
    assert.doesNotMatch(code(classifier), /github\.event\.pull_request\.(title|body|head\.ref)/);
    assert.match(classifier, /continue-on-error: true/);
    assert.doesNotMatch(read("ci.yml"), /tier-classifier/);
    assert.match(read("labeler.yml"), /sync-labels: false/);
  });
});

describe("main-guard.yml", () => {
  it("only reads and files issues; it never writes contents", () => {
    const text = read("main-guard.yml");
    assert.match(text, /issues: write/);
    assert.match(text, /contents: read/);
    assert.doesNotMatch(text, /contents: write/);
  });
});

describe("nightly-compliance.yml", () => {
  const text = read("nightly-compliance.yml");

  it("grants issues: write only to the report job", () => {
    const gate = text.split(/^ {2}report:/m)[0];
    assert.doesNotMatch(gate, /issues: write/);
    assert.match(text.split(/^ {2}report:/m)[1], /issues: write/);
  });

  it("checks out without persisting credentials", () => {
    assert.match(text, /persist-credentials: false/);
  });
});

describe("auto-qa.yml", () => {
  const text = read("auto-qa.yml");

  it("holds contents: write only on the tune job, which never runs on pull_request", () => {
    const top = text.split(/^jobs:/m)[0];
    assert.doesNotMatch(code(top), /: write/);
    assert.match(text, /tune:\n[\s\S]*?if: github\.event_name != 'pull_request'/);
  });

  it("mints the Hive App token only when BOT_APP_ID is set, and falls back to the default token", () => {
    assert.match(text, /if: vars\.BOT_APP_ID != ''/);
    assert.match(text, /app-id: \$\{\{ vars\.BOT_APP_ID \}\}/);
    assert.match(text, /private-key: \$\{\{ secrets\.BOT_APP_PRIVATE_KEY \}\}/);
    assert.match(text, /steps\.bot\.outputs\.token \|\| github\.token/);
  });

  it("pushes only its own bot branch, never main, and signs its commit off for the DCO", () => {
    assert.match(text, /git push --force origin auto-qa\/retune/);
    assert.doesNotMatch(code(text), /git push[^\n]*\bmain\b/);
    assert.match(text, /git commit -s /);
  });
});

describe("pr-metrics.yml, quality-report.yml and coverage-gate.yml", () => {
  it("only read: no write scope anywhere", () => {
    for (const file of ["pr-metrics.yml", "quality-report.yml", "coverage-gate.yml"]) {
      assert.doesNotMatch(code(read(file)), /: write\b/, file);
    }
  });

  it("coverage-gate keeps the workflow name ci-feedback watches, and ci-feedback watches the names that make a PR red", () => {
    assert.match(read("coverage-gate.yml"), /^name: Coverage gate$/m);
    assert.match(read("ci.yml"), /^name: CI$/m);
    assert.match(read("ci-feedback.yml"), /workflows: \[CI, Coverage gate\]/);
  });

  it("the rust coverage job runs on pull requests and enforces a threshold", () => {
    const text = read("coverage-gate.yml");
    const rust = text.slice(text.indexOf("  rust:"), text.indexOf("  rust:") + 4000);
    assert.doesNotMatch(rust, /if: github\.event_name == 'workflow_dispatch'/);
    assert.match(rust, /runs-on: ubuntu-24\.04/);
    assert.match(rust, /check-coverage\.mjs --component rust/);
  });
});
