import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { classify, globToRegExp, loadConfig, parseYamlSubset, renderComment } from "../classify-change.mjs";

const CONFIG_PATH = ".github/risk-tiers.yml";
const MIN_TEXT_LENGTH = 20;
const config = () => loadConfig(readFileSync(CONFIG_PATH, "utf8"));
const repoFiles = () =>
  execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean);

describe("globToRegExp", () => {
  it("stops * at a slash but lets ** cross directories", () => {
    assert.equal(globToRegExp("server/*.ts").test("server/a.ts"), true);
    assert.equal(globToRegExp("server/*.ts").test("server/src/a.ts"), false);
    assert.equal(globToRegExp("server/**").test("server/src/a.ts"), true);
    assert.equal(globToRegExp("**/*.md").test("README.md"), true);
  });
});

describe("parseYamlSubset", () => {
  it("rejects flow style rather than guessing", () => {
    assert.throws(() => parseYamlSubset("a: [1, 2]"), /flow sequences/);
  });

  it("rejects an unquoted value containing ': '", () => {
    assert.throws(() => parseYamlSubset("a: b: c"), /quote/);
  });

  it("rejects duplicate keys with the line where they repeat", () => {
    assert.throws(
      () => parseYamlSubset("default_tier: critical\ndefault_tier: low\n"),
      /line 2: duplicate key default_tier/,
    );
    assert.throws(
      () => parseYamlSubset("tiers:\n  - id: critical\n    rank: 4\n    rank: 1\n"),
      /line 4: duplicate key rank/,
    );
  });

  it("allows the same key in separate mappings", () => {
    assert.deepEqual(parseYamlSubset("tiers:\n  - id: critical\n  - id: high\n"), {
      tiers: [{ id: "critical" }, { id: "high" }],
    });
  });
});

describe("loadConfig", () => {
  const base = (extra) =>
    [
      "version: 1",
      "default_tier: a",
      "tiers:",
      "  - id: a",
      "    label: risk/a",
      "    rank: 1",
      "    summary: something long enough here",
      ...extra,
      "    paths:",
      "      - match: x/**",
      "        why: because reasons",
    ].join("\n");

  it("fails when a tier has no review text", () => {
    assert.throws(() => loadConfig(base([])), /review is required/);
  });

  it("reads the optional pending flag, defaulting to false", () => {
    const text = [base(["    review: a reviewer reads this"]), "      - match: y/**", "        why: not here yet", "        pending: true"].join("\n");
    const { tiers } = loadConfig(text);
    assert.deepEqual(
      tiers[0].paths.map((p) => p.pending),
      [false, true],
    );
  });
});

describe(".github/risk-tiers.yml", () => {
  it("parses and every tier has a summary, a review requirement and paths", () => {
    const { tiers } = config();
    assert.ok(tiers.length >= 4);
    for (const tier of tiers) {
      assert.match(tier.label, /^risk\//, `tier ${tier.id} label`);
      assert.ok(tier.summary.length > MIN_TEXT_LENGTH, `tier ${tier.id} has no summary`);
      assert.ok(tier.review.length > MIN_TEXT_LENGTH, `tier ${tier.id} has no review text`);
      assert.ok(tier.paths.length > 0, `tier ${tier.id} lists no paths`);
      for (const entry of tier.paths) {
        assert.ok(entry.why.length > MIN_TEXT_LENGTH, `${entry.match} has no reason`);
      }
    }
  });

  it("gives every tier a distinct id, label and rank", () => {
    const { tiers } = config();
    for (const key of ["id", "label", "rank"]) {
      assert.equal(new Set(tiers.map((t) => t[key])).size, tiers.length, key);
    }
  });

  it("lists no non-pending pattern that matches nothing in the repository", () => {
    // A renamed file would otherwise be silently downgraded to the default tier.
    const files = repoFiles();
    for (const tier of config().tiers) {
      for (const entry of tier.paths) {
        if (entry.pending) continue;
        assert.ok(
          files.some((f) => entry.re.test(f)),
          `${tier.id}: pattern "${entry.match}" matches no file (mark it pending: true if the file does not exist yet)`,
        );
      }
    }
  });

  it("rates auth, limits, the config that names commands, the Hermit backend, schemas, the bundle format, installers and workflows as critical", () => {
    const files = repoFiles();
    const mustBeCritical = [
      "crates/hairspring-server/src/auth.rs",
      "crates/hairspring-server/src/ratelimit.rs",
      "crates/hairspring-server/src/worker_api.rs",
      "crates/hairspring-engine/src/config.rs",
      "crates/hairspring-hermit/src/exec.rs",
      "crates/hairspring-store/src/sqlite_schema.rs",
      "crates/hairspring-store/src/pg_schema.rs",
      "crates/hairspring-core/src/bundle.rs",
      "scripts/install.sh",
      "scripts/install-hermit.sh",
      "scripts/hermit.pin",
      "integrations/github-action/action.yml",
      "Dockerfile",
      "SECURITY.md",
      ".github/workflows/release.yml",
      ".github/workflows/ci.yml",
      ".github/risk-tiers.yml",
      "AGENTS.md",
      "CLAUDE.md",
      ".claude/settings.json",
      "docs/security/SECURITY-AI.md",
    ];
    for (const file of mustBeCritical) {
      assert.ok(files.includes(file), `${file} does not exist, so the case is fiction`);
      assert.equal(classify(config(), [file]).tier.id, "critical", `${file} should be critical`);
    }
  });

  it("rates other Rust code, the SDKs, manifests and scripts as high, the web UI as standard, docs as low", () => {
    const cfg = config();
    assert.equal(classify(cfg, ["crates/hairspring-core/src/explore.rs"]).tier.id, "high");
    assert.equal(classify(cfg, ["crates/hairspring-service/src/lib.rs"]).tier.id, "high");
    assert.equal(classify(cfg, ["sdks/go/run.go"]).tier.id, "high");
    assert.equal(classify(cfg, ["Cargo.lock"]).tier.id, "high");
    assert.equal(classify(cfg, ["web/package-lock.json"]).tier.id, "high");
    assert.equal(classify(cfg, ["scripts/check-site.py"]).tier.id, "high");
    assert.equal(classify(cfg, ["web/src/App.tsx"]).tier.id, "standard");
    assert.equal(classify(cfg, ["examples/glibc-condlock/README.md"]).tier.id, "standard");
    assert.equal(classify(cfg, ["docs/architecture.md"]).tier.id, "low");
    assert.equal(classify(cfg, ["CHANGELOG.md"]).tier.id, "low");
  });
});

describe("classify", () => {
  it("takes the highest tier any file matched, not the first", () => {
    assert.equal(classify(config(), ["docs/x.md", "web/src/App.tsx"]).tier.id, "standard");
    assert.equal(classify(config(), ["docs/x.md", "crates/hairspring-server/src/auth.rs"]).tier.id, "critical");
  });

  it("falls back to the default tier and says so", () => {
    const result = classify(config(), ["some/new/dir/file.xyz"]);
    assert.equal(result.matchedDefault, true);
    assert.equal(result.tier.id, config().defaultTier.id);
    assert.deepEqual(result.matches, []);
  });

  it("normalises separators, whitespace and leading ./", () => {
    for (const raw of ["crates\\hairspring-server\\src\\auth.rs", " crates/hairspring-server/src/auth.rs ", "./crates/hairspring-server/src/auth.rs"]) {
      assert.equal(classify(config(), [raw]).tier.id, "critical", raw);
    }
  });

  it("reports the reason that decided each file, not every overlapping pattern", () => {
    const result = classify(config(), ["crates/hairspring-server/src/auth.rs"]);
    assert.equal(result.matches.length, 1);
    assert.equal(result.matches[0].tier, "critical");
  });
});

describe("renderComment", () => {
  it("starts with the marker the workflow upserts on and names tier, review and reason", () => {
    const body = renderComment(classify(config(), ["crates/hairspring-server/src/auth.rs"]));
    assert.ok(body.startsWith("<!-- change-classification -->"));
    assert.ok(body.includes("`critical`"));
    assert.ok(body.includes("Review requirement."));
    assert.ok(body.includes("crates/hairspring-server/src/auth.rs"));
  });
});

describe("classify-change.mjs CLI", () => {
  it("reads a file of paths and prints JSON with the tier and label", () => {
    const dir = mkdtempSync(path.join(tmpdir(), "classify-"));
    const list = path.join(dir, "changed.txt");
    writeFileSync(list, "docs/x.md\ncrates/hairspring-server/src/auth.rs\n");

    const out = execFileSync("node", ["scripts/classify-change.mjs", "--config", CONFIG_PATH, "--files", list], {
      encoding: "utf8",
    });

    const parsed = JSON.parse(out);
    assert.equal(parsed.tier, "critical");
    assert.equal(parsed.label, "risk/critical");
    assert.equal(parsed.files, 2);
  });

  it("exits non-zero without --files", () => {
    assert.throws(() => execFileSync("node", ["scripts/classify-change.mjs"], { encoding: "utf8", stdio: "pipe" }));
  });
});

describe("classifier is path-only and advisory", () => {
  it("never reads the environment or the event payload", () => {
    const source = readFileSync("scripts/classify-change.mjs", "utf8");
    assert.doesNotMatch(source, /process\.env/);
    assert.doesNotMatch(source, /github\.event/);
  });

  it("the workflow takes no PR text and is not wired into ci.yml", () => {
    const workflow = readFileSync(".github/workflows/tier-classifier.yml", "utf8");
    assert.doesNotMatch(workflow, /pull_request_target/);
    assert.doesNotMatch(workflow, /github\.event\.pull_request\.(title|body|head\.ref)/);
    assert.doesNotMatch(readFileSync(".github/workflows/ci.yml", "utf8"), /tier-classifier/);
  });
});
