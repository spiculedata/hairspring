import type { MeState } from '../me';
import type { Worker, WorkerJob } from '../types';
import { CAPS_NOT_STATED, CAPS_UNREADABLE, capabilityMore } from './workersWording';

/** Lines of capabilities shown for one worker, and characters in one line. The server limits the text to 4 KiB. */
export const MAX_CAPABILITY_LINES = 8;
export const MAX_CAPABILITY_TEXT = 120;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

const cut = (text: string): string => (text.length > MAX_CAPABILITY_TEXT ? `${text.slice(0, MAX_CAPABILITY_TEXT)}…` : text);

function valueText(value: unknown): string {
  if (Array.isArray(value) && value.every((v) => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean')) {
    return value.join(', ');
  }
  if (typeof value === 'string') return value;
  return JSON.stringify(value) ?? String(value);
}

/**
 * The capabilities a worker stated, one line per key. Text only: the caller puts each line in a text node, so nothing
 * the worker sent is read as markup. A worker's own words are cut, never trusted to be short.
 */
export function capabilityLines(worker: Worker): string[] {
  const caps = worker.capabilities;
  // `null` is sent for text that is not JSON; the text `null` is valid JSON and says nothing.
  if (caps === null) return [worker.caps_json?.trim() === 'null' ? CAPS_NOT_STATED : CAPS_UNREADABLE];
  if (caps === undefined) return [CAPS_NOT_STATED];
  if (!isRecord(caps)) return [cut(JSON.stringify(caps) ?? String(caps))];
  const entries = Object.entries(caps);
  if (entries.length === 0) return [CAPS_NOT_STATED];
  const lines = entries.slice(0, MAX_CAPABILITY_LINES).map(([key, value]) => cut(`${key}: ${valueText(value)}`));
  const left = entries.length - lines.length;
  return left > 0 ? [...lines, capabilityMore(left)] : lines;
}

/** The jobs named for a worker and how many more it holds than are named. */
export function jobsSummary(worker: Worker): { jobs: readonly WorkerJob[]; more: number } {
  const jobs = worker.current_jobs ?? [];
  return { jobs, more: Math.max(0, (worker.leased_jobs ?? 0) - jobs.length) };
}

/** The id is a credential, so the server sends it only to the run role and above. The column follows what it sent. */
export const idColumnShown = (workers: readonly Worker[]): boolean => workers.some((w) => w.id !== undefined);

/** Say why there is no id column, but only when the server has said the caller's role is read. */
export const hiddenIdNote = (me: MeState): boolean => me.status === 'ready' && !me.me.can.run;
