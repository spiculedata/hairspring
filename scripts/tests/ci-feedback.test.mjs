// The CI feedback comment behind scripts/ci-feedback.mjs and ci-feedback.yml.
//
// Everything in the script is a pure function over plain objects with `gh` passed
// in, so none of this needs a token or a network. The cases are the ones that
// would quietly make the comment useless or unsafe if they were wrong:
//
//   - a log line carrying backticks must not break out of the code fence
//   - only the newest run of a workflow counts, or a fixed failure stays red
//   - a run still going must not be reported green
//   - the comment is edited in place, never duplicated
//   - the workflow itself must stay safe: no PR checkout, no fork runs, no
//     approval, and write access to nothing but the one comment
//

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { describe, it } from "node:test";
// @ts-expect-error -- a plain .mjs module, so it has no type declarations to import.
import { MARKER, buildComment, buildResolved, cleanLine, collect, excerpt, fetchLog, fence, findComment, run, upsert } from "../ci-feedback.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const SHA = "0123456789abcdef0123456789abcdef01234567";
const REPO = "spiculedata/kinetic";

describe("cleaning a log line", () => {
  it("drops colour codes and the timestamp", () => {
    assert.equal(cleanLine("2026-10-13T14:47:26.3651333Z \u001b[1m\u001b[91merror\u001b[0m: boom"), "error: boom");
  });

  it("bounds a very long line", () => {
    assert.ok(cleanLine("x".repeat(1000)).length < 400);
  });
});

describe("choosing the lines that explain a failure", () => {
  it("keeps rustc errors and rustfmt diffs, in order, and drops the noise", () => {
    const log = [
      "2026-10-13T14:00:00.0Z Compiling app v0.1.0",
      "2026-10-13T14:00:01.0Z \u001b[91merror[E0658]\u001b[0m: cannot call conditionally-const operator in constants",
      "2026-10-13T14:00:02.0Z Diff in /home/runner/work/keel/src/observability.rs:94:",
      "2026-10-13T14:00:03.0Z ##[error]Process completed with exit code 101.",
    ].join("\n");
    assert.deepEqual(excerpt(log), [
      "error[E0658]: cannot call conditionally-const operator in constants",
      "Diff in /home/runner/work/keel/src/observability.rs:94:",
    ]);
  });

  it("shows the generic exit-code line only when nothing better exists", () => {
    assert.deepEqual(excerpt("##[error]Process completed with exit code 1."), [
      "##[error]Process completed with exit code 1.",
    ]);
  });

  it("falls back to the tail of the log when no line looks like an error", () => {
    const log = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
    const lines = excerpt(log);
    assert.equal(lines.length, 15);
    assert.equal(lines.at(-1), "line 39");
  });

  it("caps how many lines one job contributes and does not repeat a line", () => {
    const log = Array.from({ length: 100 }, (_, i) => `error: failure number ${i}`).join("\n") + "\nerror: failure number 0";
    assert.equal(excerpt(log).length, 30);
  });

  it("copes with a missing log", () => {
    assert.deepEqual(excerpt(undefined), []);
  });
});

describe("quoting log text safely", () => {
  it("uses a fence longer than any backtick run inside the text", () => {
    const quoted = fence("a ``` b ````` c");
    assert.equal(quoted.startsWith("``````\n"), true);
    assert.equal(quoted.endsWith("\n``````"), true);
  });

  it("uses the usual three backticks when the text has none", () => {
    assert.equal(fence("plain"), "```\nplain\n```");
  });
});

describe("the comment", () => {
  const failures = [
    {
      workflow: "CI",
      url: "https://github.com/x/actions/runs/1",
      jobs: [{ name: "Rust core", url: "https://github.com/x/job/9", lines: ["error: nope"] }],
    },
  ];

  it("is marked, names the commit, the job and the reason, and links the log", () => {
    const body = buildComment({ sha: SHA, failures });
    assert.equal(body.startsWith(MARKER), true);
    assert.ok(body.includes("`0123456`"));
    assert.ok(body.includes("### Rust core"));
    assert.ok(body.includes("error: nope"));
    assert.ok(body.includes("https://github.com/x/job/9"));
  });

  it("stays under GitHub's size limit however much fails, and says it left some out", () => {
    const big = Array.from({ length: 200 }, (_, i) => ({
      name: `job ${i}`,
      url: "https://github.com/x/job",
      lines: Array.from({ length: 30 }, () => "e".repeat(290)),
    }));
    const body = buildComment({ sha: SHA, failures: [{ workflow: "CI", url: "u", jobs: big }] });
    assert.ok(body.length < 65000);
    assert.ok(body.includes("left out"));
  });

  it("still points at the run when a workflow failed with no failing job", () => {
    const body = buildComment({ sha: SHA, failures: [{ workflow: "CI", url: "https://run", jobs: [] }] });
    assert.ok(body.includes("https://run"));
  });

  it("the green version carries the same marker so it replaces the failure", () => {
    assert.equal(buildResolved({ sha: SHA }).startsWith(MARKER), true);
  });
});

/** A fake `gh`: routes on the API path and records every call. */
function fakeGh(routes                                                                                                            ) {
  const calls             = [];
  const gh = (args          )         => {
    calls.push(args);
    const url = args.find((a) => a.startsWith("repos/")) ?? "";
    if (url.includes("/actions/runs?")) return JSON.stringify({ workflow_runs: routes.runs ?? [] });
    const jobs = url.match(/\/actions\/runs\/(\d+)\/jobs/);
    if (jobs) return JSON.stringify({ jobs: routes.jobs?.[Number(jobs[1])] ?? [] });
    const logs = url.match(/\/actions\/jobs\/(\d+)\/logs/);
    if (logs) {
      const text = routes.logs?.[Number(logs[1])];
      if (text === undefined) throw new Error("log expired");
      return text;
    }
    if (url.includes("/issues/") && url.includes("/comments?")) return routes.commentIds ?? "";
    return "";
  };
  return { gh, calls };
}

const runRow = (id        , name        , conclusion        , created        , status = "completed") => ({
  id,
  name,
  conclusion,
  status,
  created_at: created,
  html_url: `https://github.com/x/actions/runs/${id}`,
});

describe("collecting what failed", () => {
  it("lists failed jobs with their log excerpt and ignores jobs that passed", () => {
    const { gh } = fakeGh({
      runs: [runRow(1, "CI", "failure", "2026-10-13T10:00:00Z")],
      jobs: {
        1: [
          { id: 11, name: "e2e", conclusion: "failure", html_url: "https://j/11" },
          { id: 12, name: "lint", conclusion: "success", html_url: "https://j/12" },
        ],
      },
      logs: { 11: "error: the thing broke" },
    });
    const { failures, pending } = collect({ gh, repo: REPO, sha: SHA });
    assert.equal(pending, false);
    assert.equal(failures.length, 1);
    assert.deepEqual(failures[0].jobs.map((j                  ) => j.name), ["e2e"]);
    assert.deepEqual(failures[0].jobs[0].lines, ["error: the thing broke"]);
  });

  it("counts only the newest run of a workflow, so a fixed failure is not reported", () => {
    const { gh } = fakeGh({
      runs: [runRow(1, "CI", "failure", "2026-10-13T10:00:00Z"), runRow(2, "CI", "success", "2026-10-13T11:00:00Z")],
    });
    assert.deepEqual(collect({ gh, repo: REPO, sha: SHA }).failures, []);
  });

  it("reports runs still going as pending", () => {
    const { gh } = fakeGh({ runs: [runRow(1, "CI", "", "2026-10-13T10:00:00Z", "in_progress")] });
    assert.equal(collect({ gh, repo: REPO, sha: SHA }).pending, true);
  });

  it("keeps the job and its link when its log has expired", () => {
    const { gh } = fakeGh({
      runs: [runRow(1, "CI", "failure", "2026-10-13T10:00:00Z")],
      jobs: { 1: [{ id: 11, name: "e2e", conclusion: "failure", html_url: "https://j/11" }] },
    });
    const job = collect({ gh, repo: REPO, sha: SHA }).failures[0].jobs[0];
    assert.equal(job.url, "https://j/11");
    assert.deepEqual(job.lines, []);
  });

  it("gathers failures across both workflows", () => {
    const { gh } = fakeGh({
      runs: [runRow(1, "CI", "failure", "2026-10-13T10:00:00Z"), runRow(2, "Coverage gate", "failure", "2026-10-13T10:00:00Z")],
    });
    assert.deepEqual(collect({ gh, repo: REPO, sha: SHA }).failures.map((f                      ) => f.workflow).sort(), [
      "CI",
      "Coverage gate",
    ]);
  });

  it("ignores unrelated failed and pending workflows on the same commit", () => {
    const { gh } = fakeGh({
      runs: [
        runRow(1, "CI", "success", "2026-10-13T10:00:00Z"),
        runRow(2, "Coverage gate", "success", "2026-10-13T10:01:00Z"),
        runRow(3, "quality-report", "failure", "2026-10-13T10:02:00Z"),
        runRow(4, "auto-review", "", "2026-10-13T10:03:00Z", "in_progress"),
      ],
    });
    assert.deepEqual(collect({ gh, repo: REPO, sha: SHA }), { failures: [], pending: false });
  });
});

describe("posting the comment", () => {
  it("creates one when none exists", () => {
    const { gh, calls } = fakeGh({});
    assert.equal(upsert({ gh, repo: REPO, pr: "309", body: "b", existing: null }), "created");
    assert.ok(calls.at(-1).includes("POST"));
  });

  it("edits the existing one in place", () => {
    const { gh, calls } = fakeGh({});
    assert.equal(upsert({ gh, repo: REPO, pr: "309", body: "b", existing: "555" }), "updated");
    assert.ok(calls.at(-1).includes("PATCH"));
    assert.ok(calls.at(-1).includes(`repos/${REPO}/issues/comments/555`));
  });

  it("finds the comment by its marker", () => {
    const { gh } = fakeGh({ commentIds: "777\n888\n" });
    assert.equal(findComment({ gh, repo: REPO, pr: "309" }), "777");
    assert.equal(findComment({ gh: fakeGh({}).gh, repo: REPO, pr: "309" }), null);
  });
});

describe("what a run does", () => {
  const failing = {
    runs: [runRow(1, "CI", "failure", "2026-10-13T10:00:00Z")],
    jobs: { 1: [{ id: 11, name: "e2e", conclusion: "failure", html_url: "https://j/11" }] },
    logs: { 11: "error: x" },
  };

  it("posts the failure", () => {
    const { gh, calls } = fakeGh(failing);
    assert.equal(run({ gh, repo: REPO, sha: SHA, pr: "309" }), "created");
    assert.ok(calls.at(-1)?.join(" ").includes("CI failed"));
  });

  it("marks an existing failure comment green once everything passes", () => {
    const { gh, calls } = fakeGh({ runs: [runRow(1, "CI", "success", "2026-10-13T10:00:00Z")], commentIds: "555" });
    assert.equal(run({ gh, repo: REPO, sha: SHA, pr: "309" }), "updated");
    assert.ok(calls.at(-1)?.join(" ").includes("CI is green"));
  });

  it("clears a stale failure comment despite an unrelated failed workflow", () => {
    const { gh, calls } = fakeGh({
      runs: [
        runRow(1, "CI", "success", "2026-10-13T10:00:00Z"),
        runRow(2, "Coverage gate", "success", "2026-10-13T10:01:00Z"),
        runRow(3, "quality-report", "failure", "2026-10-13T10:02:00Z"),
      ],
      commentIds: "555",
    });
    assert.equal(run({ gh, repo: REPO, sha: SHA, pr: "309" }), "updated");
    assert.ok(calls.at(-1)?.join(" ").includes("CI is green"));
  });

  it("never opens a comment just to say all is well", () => {
    const { gh, calls } = fakeGh({ runs: [runRow(1, "CI", "success", "2026-10-13T10:00:00Z")] });
    assert.equal(run({ gh, repo: REPO, sha: SHA, pr: "309" }), "nothing to do");
    assert.equal(calls.some((c) => c.includes("POST") || c.includes("PATCH")), false);
  });

  it("does not call a commit green while a workflow is still running", () => {
    const { gh } = fakeGh({
      runs: [runRow(1, "CI", "success", "2026-10-13T10:00:00Z"), runRow(2, "Coverage gate", "", "2026-10-13T10:00:00Z", "in_progress")],
      commentIds: "555",
    });
    assert.equal(run({ gh, repo: REPO, sha: SHA, pr: "309" }), "nothing to do");
  });
});

describe("the workflow stays safe", () => {
  const wf = readFileSync(path.join(root, ".github/workflows/ci-feedback.yml"), "utf8");

  it("runs from workflow_run on the two red-making workflows", () => {
    assert.match(wf, /workflow_run:\s*\n\s*workflows: \[CI, Coverage gate\]/);
  });

  it("never checks out the pull request or uses pull_request_target", () => {
    assert.doesNotMatch(wf, /pull_request_target/);
    assert.doesNotMatch(wf, /ref:\s*\$\{\{\s*github\.event\.workflow_run\.head/);
    assert.doesNotMatch(wf, /github\.event\.pull_request\.head/);
  });

  it("skips fork pull requests", () => {
    assert.ok(wf.includes("github.event.workflow_run.head_repository.full_name == github.repository"));
  });

  it("asks for exactly read on actions and contents and write on pull requests", () => {
    const block = wf.match(/\npermissions:\n((?: {2}.*\n)+)/)?.[1] ?? "";
    assert.deepEqual(block.trim().split("\n").map((l) => l.trim()).sort(), [
      "actions: read",
      "contents: read",
      "pull-requests: write",
    ]);
  });

  it("does not approve runs or reviews", () => {
    assert.doesNotMatch(wf, /\/approve\b/);
    assert.doesNotMatch(wf, /gh pr review|--approve/);
  });

  it("does not use the bot's own comment trigger phrase", () => {
    const script = readFileSync(path.join(root, "scripts/ci-feedback.mjs"), "utf8");
    assert.doesNotMatch(script, /\/ai-fix/);
  });
});

describe("fetching a job log", () => {
  it("asks gh to print colour codes, and retries without the flag on an older gh", () => {
    const seen = [];
    const gh = (args) => {
      seen.push(args);
      if (args.includes("--allow-escape-sequences")) throw new Error("unknown flag: --allow-escape-sequences");
      return "error: boom";
    };
    assert.equal(fetchLog({ gh, repo: "o/r", jobId: 7 }), "error: boom");
    assert.equal(seen.length, 2);
    assert.ok(seen[0].includes("--allow-escape-sequences"));
    assert.ok(!seen[1].includes("--allow-escape-sequences"));
  });
});
