// Static guards over the two AI workflows. Hermetic: reads workflow text only.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

const DIR = ".github/workflows";
const CLAUDE_ACTION = "anthropics/claude-code-action@1d6de8cb0c237e7c15e9e1bdf973826ebae490cc # v1.0.248";
const read = (name) => readFileSync(`${DIR}/${name}`, "utf8");
const code = (text) => text.split("\n").filter((l) => !l.trim().startsWith("#")).join("\n");

describe("ai-fix.yml", () => {
  const text = read("ai-fix.yml");

  it("is inert without a secret and checks the label sender's repository permission", () => {
    assert.match(text, /CLAUDE_CODE_OAUTH_TOKEN/);
    assert.match(text, /ANTHROPIC_API_KEY/);
    assert.match(text, /is not configured/);
    assert.match(text, /collaborators\/\$SENDER\/permission/);
    assert.match(text, /admin\|maintain\|write/);
  });

  it("only reacts to its own label and rejects bots unless AI_FIX_ALLOWED_BOTS lists them", () => {
    assert.match(text, /github\.event\.label\.name == 'ai-fix-requested'/);
    assert.match(text, /vars\.AI_FIX_ALLOWED_BOTS/);
    assert.match(text, /SENDER_TYPE" = "Bot"/);
    assert.match(text, /\*"\[bot\]"\|\*-bot/);
  });

  it("pins the Claude action to the reviewed commit and bounds turns and minutes", () => {
    assert.ok(text.includes(CLAUDE_ACTION), "claude-code-action is not pinned to the reviewed SHA");
    assert.match(text, /--max-turns \d+/);
    assert.match(text, /timeout-minutes: \d+/);
  });

  it("uses a tool allowlist and denies force-push, pushes to main and network tools", () => {
    assert.match(text, /--allowedTools/);
    for (const denied of ["git push --force", "git push -f", "git push origin main", "curl", "wget"]) {
      assert.ok(text.includes(`Bash(${denied}`), `ai-fix.yml does not deny ${denied}`);
    }
    assert.doesNotMatch(text.match(/--allowedTools "[^"]*"/)[0], /Bash\((curl|wget|gh pr merge|gh pr review|rm|bash|sh)\b/);
  });

  it("denies edits to the gates, the installers and the critical-tier Rust files", () => {
    const denied = text.match(/--disallowedTools "[^"]*"/)[0];
    for (const path of [
      "/.github/**",
      "/.claude/**",
      "/docs/security/**",
      "/SECURITY.md",
      "/AGENTS.md",
      "/scripts/install.sh",
      "/scripts/hermit.pin",
      "/integrations/github-action/**",
      "/crates/hairspring-server/src/auth.rs",
      "/crates/hairspring-engine/src/config.rs",
      "/crates/hairspring-hermit/**",
      "/crates/hairspring-store/src/sqlite_schema.rs",
      "/crates/hairspring-core/src/bundle.rs",
    ]) {
      assert.ok(denied.includes(`Edit(${path})`), `ai-fix.yml does not deny edits to ${path}`);
    }
  });

  it("grants write access only on the job, never at the top level, and installs without scripts", () => {
    assert.match(text, /^permissions: \{\}/m);
    assert.match(text, /^ {6}contents: write/m);
    assert.match(text, /npm ci --ignore-scripts/);
  });

  it("tells the agent to sign off its commits (this repository requires the DCO)", () => {
    assert.match(text, /git commit -s/);
  });

  it("never checks out a pull request: it runs on issues, from the default branch", () => {
    assert.doesNotMatch(code(text), /pull_request/);
    assert.doesNotMatch(code(text), /ref:\s*\$\{\{/);
  });
});

describe("auto-review.yml", () => {
  const text = read("auto-review.yml");
  const body = code(text);

  it("is disabled, and a stub: no checkout, no install, no agent, no write scope", () => {
    assert.match(body, /if: github\.event\.pull_request\.number == 0/);
    assert.doesNotMatch(body, /actions\/checkout/);
    assert.doesNotMatch(body, /claude-code-action/);
    assert.doesNotMatch(body, /npm ci|cargo |pip install/);
    assert.doesNotMatch(body, /: write\b/);
    assert.doesNotMatch(body, /head\.ref|head\.sha|head\.repo/);
    assert.match(body, /^permissions: \{\}/m);
  });

  it("explains why in its header", () => {
    assert.match(text, /DISABLED/);
  });
});
