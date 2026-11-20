// Static checks on how this repository wires the shared acceptance-spec workflows. Hermetic: reads workflow text only.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { describe, it } from "node:test";

const DIR = ".github/workflows";
const read = (name) => readFileSync(`${DIR}/${name}`, "utf8");
const code = (text) => text.split("\n").filter((l) => !l.trim().startsWith("#")).join("\n");
const SHARED = /spiculedata\/acceptance-spec\/\.github\/workflows\/[\w-]+\.yml@(\S+)/g;
const pins = () =>
  readdirSync(DIR)
    .filter((f) => f.endsWith(".yml"))
    .flatMap((file) => [...read(file).matchAll(SHARED)].map((m) => ({ file, ref: m[1] })));

describe("acceptance-spec wiring", () => {
  it("pins every caller to a full commit SHA", () => {
    assert.ok(pins().length >= 2, "thin-caller-gate and pin-health");
    for (const { file, ref } of pins()) assert.match(ref, /^[0-9a-f]{40}$/, `${file} pins a full SHA`);
  });

  it("uses one SHA everywhere, and the README names the same one", () => {
    const shas = new Set(pins().map((p) => p.ref));
    assert.equal(shas.size, 1, `callers pin different SHAs: ${[...shas].join(", ")}`);
    assert.ok(readFileSync("acceptance/README.md", "utf8").includes([...shas][0]), "acceptance/README.md names the pinned SHA");
  });

  it("gives each caller the version comment the pin convention asks for", () => {
    for (const file of ["thin-caller-gate.yml", "acceptance-pin-health.yml"]) {
      assert.match(read(file), /@[0-9a-f]{40} # v1 acceptance-spec@[0-9a-f]{7}/, file);
    }
  });

  it("keeps thin-caller-gate report-only, outside ci, and not named like a ci job", () => {
    const gate = code(read("thin-caller-gate.yml"));
    assert.match(gate, /mode: report/);
    assert.doesNotMatch(gate, /mode: enforce/);
    assert.doesNotMatch(read("ci.yml"), /thin-caller-gate\.yml@/);
    assert.doesNotMatch(gate, /^\s+name: ci\s*$/m);
    assert.match(gate, /^on:\n {2}pull_request:/m);
  });

  it("keeps pin-health from opening issues, with issues: write on the caller job only", () => {
    const health = read("acceptance-pin-health.yml");
    assert.match(health, /open-issue: false/);
    assert.match(health, /pin-health:\n[\s\S]*permissions:\n\s+contents: read\n\s+issues: write/);
    assert.match(health, /schedule:/);
    assert.match(health, /workflow_dispatch:/);
    assert.doesNotMatch(health.split(/^jobs:/m)[0], /issues: write/);
  });

  it("does not yet call the blocking shared workflows, and says so", () => {
    assert.doesNotMatch(read("ci.yml"), /spiculedata\/acceptance-spec/);
    assert.match(readFileSync("acceptance/README.md", "utf8"), /not wired/i);
  });

  it("has no separate acceptance.yml and no local copy of a shared script", () => {
    assert.equal(existsSync(`${DIR}/acceptance.yml`), false);
    for (const name of ["acceptance-runner.mjs", "acceptance-check.mjs", "acceptance-base-check.mjs"]) {
      assert.equal(existsSync(`scripts/${name}`), false, name);
    }
  });
});
