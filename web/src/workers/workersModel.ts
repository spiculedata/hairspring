import type { MeState } from '../me';
import type { Worker, WorkerJob } from '../types';
import { CAPS_NOT_REPORTED, CAPS_NOT_STATED, CAPS_UNREADABLE, capabilityMore } from './workersWording';

/** Lines of capabilities shown for one worker, and characters in one line. The server limits the text to 4 KiB. */
export const MAX_CAPABILITY_LINES = 8;
export const MAX_CAPABILITY_TEXT = 120;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Cut by code point, so a character outside the basic plane is never split in two. */
const cut = (text: string): string => {
  const points = Array.from(text);
  return points.length > MAX_CAPABILITY_TEXT ? `${points.slice(0, MAX_CAPABILITY_TEXT).join('')}…` : text;
};

const limitLines = (lines: readonly string[]): string[] => {
  const shown = lines.slice(0, MAX_CAPABILITY_LINES).map(cut);
  const left = lines.length - shown.length;
  return left > 0 ? [...shown, capabilityMore(left)] : shown;
};

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
  // An older server sends only the text it was given; show that text rather than claim nothing was stated.
  if (caps === undefined) {
    const text = worker.caps_json?.trim() ?? '';
    return text === '' ? [CAPS_NOT_REPORTED] : limitLines(text.split(/\r?\n/));
  }
  if (!isRecord(caps)) return [cut(JSON.stringify(caps) ?? String(caps))];
  const entries = Object.entries(caps);
  if (entries.length === 0) return [CAPS_NOT_STATED];
  return limitLines(entries.map(([key, value]) => `${key}: ${valueText(value)}`));
}

/** An older server sends neither held-job field, and a worker that holds jobs would look idle if that were read as none. */
export const holdingReported = (worker: Worker): boolean => worker.current_jobs !== undefined || worker.leased_jobs !== undefined;

/** The jobs named for a worker and how many more it holds than are named. */
export function jobsSummary(worker: Worker): { jobs: readonly WorkerJob[]; more: number } {
  const jobs = worker.current_jobs ?? [];
  return { jobs, more: Math.max(0, (worker.leased_jobs ?? 0) - jobs.length) };
}

/** The id is a credential, so the server sends it only to the run role and above. The column follows what it sent. */
export const idColumnShown = (workers: readonly Worker[]): boolean => workers.some((w) => w.id !== undefined);

/** Say why there is no id column, but only when the server has said the caller's role is read. */
export const hiddenIdNote = (me: MeState): boolean => me.status === 'ready' && !me.me.can.run;
