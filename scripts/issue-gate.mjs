// The rule behind .github/workflows/issue-gate.yml: an issue may carry `issue-approved` (the
// only label that admits work to an agent) only if it states how "done" is verified, either an
// "Acceptance criteria" heading or at least one markdown task-list item.
//
// The workflow has no checkout, so it inlines these two regular expressions. The test in
// scripts/tests/issue-gate.test.mjs reads the workflow and fails if they drift from here.

export const HEADING_PATTERN = /^#{1,6}[ \t]+acceptance criteria\b/im;
export const TASK_PATTERN = /^[ \t]*[-*][ \t]+\[[ xX]\]/m;

export function hasAcceptanceCriteria(body) {
  if (typeof body !== "string" || body.trim() === "") return false;
  return HEADING_PATTERN.test(body) || TASK_PATTERN.test(body);
}
