/**
 * Pure derivations for the failure page: which properties failed, where their witnesses sit, and how each process and
 * the event health read. No rendering here, and nothing is mutated.
 *
 * Ordinals are per thread. A witness says "thread T's k-th evaluation", never a position in time, and the lane position
 * below is a fraction of that one thread's own count.
 */
import type { KeptHealth, KeptProc, KeptProperty, KeptThread, Witness } from './types';

export type FailureReason = 'always_false' | 'unreachable_hit';

export interface FailingProperty {
  readonly property: KeptProperty;
  readonly reason: FailureReason;
  /** False evaluations (always) or hits (unreachable). */
  readonly count: number;
  readonly witness: Witness | null;
}

/** Which kept properties failed in this trial, by the same rule the server uses: its kind and counts alone. */
export function failingProperties(properties: readonly KeptProperty[]): FailingProperty[] {
  return properties.flatMap((property): FailingProperty[] => {
    const alwaysKind = property.kind === 'always' || property.kind === 'always_or_unreachable';
    if (alwaysKind && property.falses > 0) {
      return [{ property, reason: 'always_false', count: property.falses, witness: property.first_false }];
    }
    if (property.kind === 'unreachable' && property.hits > 0) {
      return [{ property, reason: 'unreachable_hit', count: property.hits, witness: property.first_true ?? property.first_false }];
    }
    return [];
  });
}

/** Where `ord` falls in a thread that made `evals` evaluations, as a fraction of that thread's own count in 0..1. */
export function lanePosition(ord: number, evals: number): number {
  if (!(evals > 0) || !(ord > 0)) return 0;
  return Math.min(1, ord / evals);
}

/** The thread report a witness names, if that process reported that thread. */
export function witnessThread(procs: readonly KeptProc[], w: Witness): KeptThread | undefined {
  return procs.find((p) => p.tag === w.proc)?.threads.find((t) => t.thread === w.thread);
}

/** How many evaluations the witness thread reported, if the witness names a thread the process reported. */
export function witnessThreadEvals(procs: readonly KeptProc[], w: Witness): number | undefined {
  return witnessThread(procs, w)?.evals;
}

const ORDINAL_TENS = 10;
const ORDINAL_HUNDREDS = 100;
const ORDINAL_TEEN_FROM = 11;
const ORDINAL_TEEN_TO = 13;
const THOUSANDS = /\B(?=(\d{3})+(?!\d))/g;

/** `2301` as `2,301st`. */
export function ordinalOf(n: number): string {
  const whole = Math.round(n);
  const last = whole % ORDINAL_TENS;
  const teen = whole % ORDINAL_HUNDREDS >= ORDINAL_TEEN_FROM && whole % ORDINAL_HUNDREDS <= ORDINAL_TEEN_TO;
  const suffix = teen ? 'th' : last === 1 ? 'st' : last === 2 ? 'nd' : last === 3 ? 'rd' : 'th';
  return `${whole.toString().replace(THOUSANDS, ',')}${suffix}`;
}

export interface LaneMark {
  readonly ord: number;
  readonly position: number;
  readonly propertyIds: readonly string[];
}

export interface LaneRow {
  readonly proc: string;
  readonly thread: string;
  readonly keyStable: boolean;
  readonly evals: number;
  readonly tape: number;
  readonly marks: readonly LaneMark[];
}

function marksFor(proc: string, thread: string, evals: number, failing: readonly FailingProperty[]): LaneMark[] {
  const byOrd = new Map<number, string[]>();
  for (const f of failing) {
    const w = f.witness;
    if (w === null || w.proc !== proc || w.thread !== thread) continue;
    byOrd.set(w.ord, [...(byOrd.get(w.ord) ?? []), f.property.id]);
  }
  return [...byOrd.entries()]
    .sort(([a], [b]) => a - b)
    .map(([ord, propertyIds]) => ({ ord, position: lanePosition(ord, evals), propertyIds }));
}

/** One row per thread of every process, in the order they were reported, each with the witnesses that sit on it. */
export function laneRows(procs: readonly KeptProc[], failing: readonly FailingProperty[]): LaneRow[] {
  return procs.flatMap((p) =>
    p.threads.map((t) => ({
      proc: p.tag,
      thread: t.thread,
      keyStable: t.key_stable,
      evals: t.evals,
      tape: t.tape_consumed,
      marks: marksFor(p.tag, t.thread, t.evals, failing),
    })),
  );
}

export interface ProcessState {
  readonly kind: 'ok' | 'missing' | 'ended';
  readonly text: string;
}

/** What a process's `end` report says, in words. */
export function processState(p: KeptProc): ProcessState {
  if (p.end === null) return { kind: 'missing', text: 'no end report' };
  const reason = p.end.reason.toLowerCase();
  const text = `ended: ${reason}, status ${p.end.status}`;
  return { kind: reason === 'exit' && p.end.status === 0 ? 'ok' : 'ended', text };
}

/** Health counters that mean event data was lost or contradictory, in the order they are shown. */
const PROBLEM_LABELS: ReadonlyArray<readonly [string, string]> = [
  ['malformed', 'malformed event lines'],
  ['too_long', 'event lines that were too long'],
  ['invalid_utf8', 'event lines that were not valid text'],
  ['over_limit', 'lines dropped by a limit'],
  ['duplicates', 'repeated hello, work, end or assert lines'],
  ['uncatalogued', 'assert lines for an id that was never catalogued'],
  ['conflicts', 'ids declared with two different kinds'],
  ['dropped_asserts', 'assert lines dropped by a kind conflict'],
  ['unreported_ids', 'catalogued ids that were never reported'],
  ['sites_mismatch', 'processes whose site count differs from their catalog'],
  ['thread_count_mismatch', 'processes whose thread count differs from their thread lines'],
  ['host_faults', 'faults the host saw while collecting events'],
];

export interface HealthProblem {
  readonly key: string;
  readonly label: string;
  readonly count: number;
}

export interface HealthSummary {
  readonly accepted: number;
  readonly problems: readonly HealthProblem[];
}

const count = (health: KeptHealth, key: string): number => {
  const n = health[key];
  return typeof n === 'number' && Number.isFinite(n) && n > 0 ? n : 0;
};

export function healthCounters(health: KeptHealth): HealthSummary {
  return {
    accepted: count(health, 'accepted'),
    problems: PROBLEM_LABELS.flatMap(([key, label]) => (count(health, key) > 0 ? [{ key, label, count: count(health, key) }] : [])),
  };
}
