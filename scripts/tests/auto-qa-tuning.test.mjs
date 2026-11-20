import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { collectRecords, main, stepToward, summarise, tune, validate } from "../auto-qa-tuner.mjs";

const committed = () => JSON.parse(readFileSync(".github/auto-qa-tuning.json", "utf8"));
const find = (cfg, id) => {
  const p = cfg.parameters.find((x) => x.id === id);
  if (!p) throw new Error(`no parameter ${id}`);
  return p;
};
const obs = (over = {}) => ({ runs: 30, passRatePct: 100, flakyRecoveryRate: 0, p95JobSeconds: 180, p95TestSeconds: 10, ...over });

describe(".github/auto-qa-tuning.json", () => {
  it("is sound as committed: every value inside its bounds", () => {
    assert.deepEqual(validate(committed()), []);
  });

  it("documents the bounds", () => {
    assert.ok((committed().bounds?.rule?.length ?? 0) > 50);
  });

  it("points at the ci workflow and a job and step that exist in it", () => {
    const { source } = committed();
    const workflow = readFileSync(`.github/workflows/${source.workflowFile}`, "utf8");
    assert.ok(workflow.includes(`name: ${source.jobName}`), `job "${source.jobName}" not found in ${source.workflowFile}`);
    assert.ok(workflow.includes(`name: ${source.testStepName}`), `step "${source.testStepName}" not found in ${source.workflowFile}`);
  });
});

describe("validate", () => {
  it("flags a value outside its bounds", () => {
    const cfg = committed();
    const broken = { ...cfg, parameters: cfg.parameters.map((p) => ({ ...p, value: p.max + 1 })) };
    assert.ok(validate(broken).length > 0);
  });

  it("rejects a config with no parameters", () => {
    assert.deepEqual(validate({ ...committed(), parameters: [] }), ["parameters must be a non-empty array"]);
  });
});

describe("stepToward", () => {
  const param = { id: "x", value: 10, baseline: 10, min: 5, max: 20, maxStep: 3, loosen: "up" };

  it("moves at most maxStep", () => {
    assert.equal(stepToward(param, 100), 13);
    assert.equal(stepToward(param, -100), 7);
  });

  it("never leaves [min, max]", () => {
    assert.equal(stepToward({ ...param, value: 19 }, 100), 20);
    assert.equal(stepToward({ ...param, value: 6 }, -100), 5);
  });
});

describe("tune", () => {
  it("changes nothing below minObservations", () => {
    const cfg = committed();
    assert.equal(tune(cfg, obs({ runs: cfg.controller.minObservations - 1, passRatePct: 0 })), cfg);
  });

  it("never edits baseline, min, max, maxStep or loosen, and does not mutate its input", () => {
    const cfg = committed();
    const before = JSON.stringify(cfg);
    const tuned = tune(cfg, obs({ passRatePct: 0, flakyRecoveryRate: 1, p95JobSeconds: 99999, p95TestSeconds: 99999 }));
    assert.equal(JSON.stringify(cfg), before);
    for (const p of tuned.parameters) {
      assert.deepEqual({ ...p, value: 0 }, { ...find(cfg, p.id), value: 0 });
    }
  });

  it("cannot loosen past the documented bound however bad the data is", () => {
    let cfg = committed();
    const worst = obs({ passRatePct: 0, flakyRecoveryRate: 1, p95JobSeconds: 999999, p95TestSeconds: 999999 });
    for (let i = 0; i < 50; i += 1) cfg = tune(cfg, worst);
    assert.equal(find(cfg, "ciTimeoutMinutes").value, find(cfg, "ciTimeoutMinutes").max);
    assert.equal(find(cfg, "testStepBudgetSeconds").value, find(cfg, "testStepBudgetSeconds").max);
    assert.equal(find(cfg, "flakeRetries").value, 1);
    assert.equal(find(cfg, "ciPassRateFloorPct").value, find(cfg, "ciPassRateFloorPct").min);
    assert.deepEqual(validate(cfg), []);
  });

  it("tightens toward the observed durations on a healthy history", () => {
    const tuned = tune(committed(), obs());
    assert.ok(find(tuned, "ciTimeoutMinutes").value < find(committed(), "ciTimeoutMinutes").value);
    assert.equal(find(tuned, "flakeRetries").value, 0);
  });

  it("caps history at historyLimit", () => {
    let cfg = committed();
    for (let i = 0; i < 20; i += 1) {
      cfg = tune(cfg, obs({ p95JobSeconds: i % 2 === 0 ? 9999 : 60 }), new Date(2026, 0, i + 1));
    }
    assert.ok(cfg.history.length <= cfg.controller.historyLimit);
  });
});

describe("summarise", () => {
  const rec = (over) => ({ conclusion: "success", runAttempt: 1, jobSeconds: 100, testSeconds: 10, ...over });

  it("computes pass rate, flaky recovery rate and p95 durations", () => {
    const s = summarise([rec({}), rec({ runAttempt: 2, jobSeconds: 300 }), rec({ conclusion: "failure" }), rec({ conclusion: "cancelled" })]);
    assert.equal(s.runs, 3);
    assert.ok(Math.abs(s.passRatePct - 66.67) < 0.1);
    assert.equal(s.flakyRecoveryRate, 0.5);
    assert.equal(s.p95JobSeconds, 300);
  });

  it("handles an empty history", () => {
    const s = summarise([]);
    assert.equal(s.runs, 0);
    assert.equal(s.passRatePct, null);
    assert.equal(s.p95JobSeconds, null);
  });
});

// --- GitHub request adapter and CLI (hermetic) --------------------------------
//
// globalThis.fetch is stubbed and the token and repository are synthetic, so nothing
// here reaches the live API. The CLI runs against a temporary copy of the config, both
// in-process (main) and as a real child process (exit codes).

const TUNER = fileURLToPath(new URL("../auto-qa-tuner.mjs", import.meta.url));
const REPO = "example-owner/example-repo";
const TOKEN = "synthetic-token-not-real";

/** Replaces globalThis.fetch with a stub answering from `routes` (API path -> JSON body). */
function stubFetch(routes, { status = 200 } = {}) {
  const calls = [];
  const original = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), headers: init?.headers });
    const body = routes[String(url).replace("https://api.github.com", "")];
    if (body === undefined) return { ok: false, status: 404, json: async () => ({}) };
    return { ok: status >= 200 && status < 300, status, json: async () => body };
  };
  return { calls, restore: () => { globalThis.fetch = original; } };
}

const runsPath = (cfg) =>
  `/repos/${REPO}/actions/workflows/${cfg.source.workflowFile}/runs?branch=${cfg.source.branch}&status=completed&per_page=${cfg.controller.windowRuns}`;
const jobsPath = (id) => `/repos/${REPO}/actions/runs/${id}/jobs?filter=latest&per_page=100`;

/** A synthetic CI history of `count` successful runs: a 120 s job with a 10 s test step. */
function ciRoutes(cfg, count) {
  const runs = [];
  const routes = { [runsPath(cfg)]: { workflow_runs: runs } };
  for (let id = 1; id <= count; id += 1) {
    runs.push({ id, conclusion: "success", run_attempt: 1 });
    routes[jobsPath(id)] = {
      jobs: [{
        name: cfg.source.jobName,
        started_at: "2026-01-01T00:00:00Z",
        completed_at: "2026-01-01T00:02:00Z",
        steps: [{ name: cfg.source.testStepName, started_at: "2026-01-01T00:00:30Z", completed_at: "2026-01-01T00:00:40Z" }],
      }],
    };
  }
  return routes;
}

describe("collectRecords via the GitHub request adapter", () => {
  it("calls the runs and jobs endpoints with the API version and bearer token headers", async () => {
    const cfg = committed();
    const fetchStub = stubFetch(ciRoutes(cfg, 2));
    try {
      const records = await collectRecords(cfg, REPO, TOKEN);
      assert.deepEqual(fetchStub.calls.map((c) => c.url), [
        `https://api.github.com${runsPath(cfg)}`,
        `https://api.github.com${jobsPath(1)}`,
        `https://api.github.com${jobsPath(2)}`,
      ]);
      for (const call of fetchStub.calls) {
        assert.deepEqual(call.headers, {
          accept: "application/vnd.github+json",
          authorization: `Bearer ${TOKEN}`,
          "x-github-api-version": "2022-11-28",
        });
      }
      assert.deepEqual(records, [
        { conclusion: "success", runAttempt: 1, jobSeconds: 120, testSeconds: 10 },
        { conclusion: "success", runAttempt: 1, jobSeconds: 120, testSeconds: 10 },
      ]);
    } finally {
      fetchStub.restore();
    }
  });

  it("rejects with the status and path on a non-2xx response", async () => {
    const cfg = committed();
    const fetchStub = stubFetch(ciRoutes(cfg, 1), { status: 503 });
    try {
      await assert.rejects(collectRecords(cfg, REPO, TOKEN), { message: `GitHub API 503 for ${runsPath(cfg)}` });
      assert.equal(fetchStub.calls.length, 1);
    } finally {
      fetchStub.restore();
    }
  });
});

describe("main (in-process, temporary working directory)", () => {
  const originalCwd = process.cwd();
  const originalEnv = { ...process.env };
  const config = committed();
  let dir;

  /** Runs main with stdout and stderr captured. */
  async function runMain(argv) {
    const out = { stdout: "", stderr: "" };
    const original = { stdout: process.stdout.write, stderr: process.stderr.write };
    process.stdout.write = (chunk) => { out.stdout += chunk; return true; };
    process.stderr.write = (chunk) => { out.stderr += chunk; return true; };
    try {
      out.code = await main(argv);
    } finally {
      process.stdout.write = original.stdout;
      process.stderr.write = original.stderr;
    }
    return out;
  }
  const configFile = () => join(dir, ".github", "auto-qa-tuning.json");
  const writeConfig = (cfg) => writeFileSync(configFile(), `${JSON.stringify(cfg, null, 2)}\n`);
  const readConfig = (path = configFile()) => JSON.parse(readFileSync(path, "utf8"));
  const writeObs = (o) => {
    const path = join(dir, "obs.json");
    writeFileSync(path, JSON.stringify(o));
    return path;
  };

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "auto-qa-tuner-"));
    mkdirSync(join(dir, ".github"));
    writeConfig(config);
    process.chdir(dir);
    delete process.env.GITHUB_TOKEN;
    delete process.env.GH_TOKEN;
    delete process.env.GITHUB_REPOSITORY;
  });

  afterEach(() => {
    process.chdir(originalCwd);
    process.env = { ...originalEnv };
    rmSync(dir, { recursive: true, force: true });
  });

  it("--check reports a sound config and changes nothing", async () => {
    const result = await runMain(["--check"]);
    assert.equal(result.code, 0);
    assert.match(result.stdout, /4 parameters, all inside their bounds/);
    assert.deepEqual(readConfig(), config);
  });

  it("--observations without --write prints the tuned config and leaves the file alone", async () => {
    const result = await runMain(["--observations", writeObs(obs())]);
    assert.equal(result.code, 0);
    const printed = JSON.parse(result.stdout);
    assert.deepEqual(printed, tune(config, obs(), new Date(printed.history[0].at)));
    assert.match(result.stderr, /retuned/);
    assert.deepEqual(readConfig(), config);
  });

  it("--observations --write rewrites the config in place, inside its bounds", async () => {
    const result = await runMain(["--observations", writeObs(obs()), "--write"]);
    assert.equal(result.code, 0);
    assert.equal(result.stdout, "");
    const written = readConfig();
    assert.deepEqual(validate(written), []);
    assert.ok(find(written, "ciTimeoutMinutes").value < find(config, "ciTimeoutMinutes").value);
    assert.equal(written.history.length, 1);
  });

  it("--observations --write --out writes elsewhere and reports no change below minObservations", async () => {
    const out = join(dir, "tuned.json");
    const result = await runMain(["--observations", writeObs(obs({ runs: 1 })), "--write", "--out", out]);
    assert.equal(result.code, 0);
    assert.match(result.stderr, /no change/);
    assert.deepEqual(readConfig(out), config);
  });

  it("--from-ci without GITHUB_TOKEN/GH_TOKEN or GITHUB_REPOSITORY fails before any request", async () => {
    const fetchStub = stubFetch({});
    try {
      await assert.rejects(runMain(["--from-ci"]), /--from-ci needs GITHUB_TOKEN \(or GH_TOKEN\) and GITHUB_REPOSITORY/);
      process.env.GH_TOKEN = TOKEN;
      await assert.rejects(runMain(["--from-ci"]), /GITHUB_REPOSITORY/);
      delete process.env.GH_TOKEN;
      process.env.GITHUB_REPOSITORY = REPO;
      await assert.rejects(runMain(["--from-ci"]), /GITHUB_TOKEN/);
      assert.equal(fetchStub.calls.length, 0);
    } finally {
      fetchStub.restore();
    }
  });

  it("--from-ci --write --out tunes from a stubbed CI history using GH_TOKEN", async () => {
    process.env.GH_TOKEN = TOKEN;
    process.env.GITHUB_REPOSITORY = REPO;
    const fetchStub = stubFetch(ciRoutes(config, config.controller.minObservations));
    const out = join(dir, "tuned.json");
    try {
      const result = await runMain(["--from-ci", "--write", "--out", out]);
      assert.equal(result.code, 0);
      assert.equal(fetchStub.calls.length, 1 + config.controller.minObservations);
      assert.ok(fetchStub.calls.every((c) => c.headers.authorization === `Bearer ${TOKEN}`));
    } finally {
      fetchStub.restore();
    }
    const tuned = readConfig(out);
    assert.deepEqual(validate(tuned), []);
    assert.equal(tuned.history[0].runs, config.controller.minObservations);
    assert.deepEqual(readConfig(), config);
  });

  it("rejects an unknown argument, no action, and an out-of-bounds config", async () => {
    await assert.rejects(runMain(["--bogus"]), /unknown argument --bogus/);
    await assert.rejects(runMain([]), /nothing to do: pass --check, --from-ci or --observations/);
    writeConfig({ ...config, parameters: config.parameters.map((p) => ({ ...p, value: p.max + 1 })) });
    await assert.rejects(runMain(["--check"]), /\.github\/auto-qa-tuning\.json is invalid/);
  });
});

describe("CLI exit behaviour (child process, temporary working directory)", () => {
  const config = committed();
  let dir;

  const cli = (args) => {
    const env = { ...process.env };
    delete env.GITHUB_TOKEN;
    delete env.GH_TOKEN;
    delete env.GITHUB_REPOSITORY;
    return spawnSync(process.execPath, [TUNER, ...args], { cwd: dir, encoding: "utf8", env, timeout: 30_000 });
  };
  const writeConfig = (cfg) => writeFileSync(join(dir, ".github", "auto-qa-tuning.json"), JSON.stringify(cfg));

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "auto-qa-tuner-cli-"));
    mkdirSync(join(dir, ".github"));
    writeConfig(config);
  });

  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  it("exits 0 for --check on a sound config", () => {
    const result = cli(["--check"]);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /all inside their bounds/);
  });

  it("exits 1 with a prefixed message for an unknown argument", () => {
    const result = cli(["--bogus"]);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /^auto-qa-tuner: unknown argument --bogus/);
  });

  it("exits 1 for --from-ci without the environment it needs", () => {
    const result = cli(["--from-ci", "--write"]);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /^auto-qa-tuner: --from-ci needs GITHUB_TOKEN/);
  });

  it("exits 1 when the config is out of bounds", () => {
    writeConfig({ ...config, parameters: config.parameters.map((p) => ({ ...p, value: p.min - 1 })) });
    const result = cli(["--check"]);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /^auto-qa-tuner: \.github\/auto-qa-tuning\.json is invalid/);
  });

  it("exits 1 when the observations file is not JSON", () => {
    writeFileSync(join(dir, "obs.json"), "not json");
    const result = cli(["--observations", "obs.json"]);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /^auto-qa-tuner: /);
  });
});
