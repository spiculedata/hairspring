import type { FailureSummary, Run } from '../types';

/** Run states that put a run in "needs attention": the two the server uses for a run that did not end well. */
const ATTENTION_STATES: readonly string[] = ['failed', 'error'];
const SAFE_NAME = /^[A-Za-z0-9._-]+$/;

/** The runs newest first, as a new array; runs created at the same time keep the order the server sent them in. */
function newestFirst(runs: readonly Run[]): Run[] {
  return runs
    .map((r, i) => ({ r, i }))
    .sort((a, b) => b.r.created_at - a.r.created_at || a.i - b.i)
    .map(({ r }) => r);
}

/** Failed or errored runs among `runs`, newest first, at most `limit`. A state the table does not know is not included. */
export function attentionRuns(runs: readonly Run[], limit: number): Run[] {
  return newestFirst(runs.filter((r) => ATTENTION_STATES.includes(r.state))).slice(0, limit);
}

/** The newest `limit` runs of any state. */
export function recentRuns(runs: readonly Run[], limit: number): Run[] {
  return newestFirst(runs).slice(0, limit);
}

/** The first target name that is safe to show inside a command (no spaces or shell characters), or undefined. */
export function firstSafeTargetName(targets: readonly { name: string }[]): string | undefined {
  return targets.find((t) => SAFE_NAME.test(t.name))?.name;
}

/** The newest `limit` failures, as a new array; failures made at the same time keep the order the server sent them in. */
export function newestFailures(failures: readonly FailureSummary[], limit: number): FailureSummary[] {
  return failures
    .map((f, i) => ({ f, i }))
    .sort((a, b) => b.f.created_at - a.f.created_at || a.i - b.i)
    .map(({ f }) => f)
    .slice(0, limit);
}
