import type { SseEvent } from './sse';
import { formatCount } from './wording';

export type Fact = readonly [label: string, value: string];

export interface EventDescription {
  readonly title: string;
  readonly facts: readonly Fact[];
}

/** Plain labels for the keys the server sends today. A key not listed here is shown as it is, with spaces for underscores. */
const LABELS: Readonly<Record<string, string>> = {
  worker: 'Worker',
  message: 'Message',
  steps: 'Steps',
  trials: 'Trials',
  state: 'State',
  distinct_schedules: 'Distinct schedules',
  inconclusive: 'Inconclusive trials',
};

const capitalise = (s: string): string => (s === '' ? s : s[0].toUpperCase() + s.slice(1));

function valueText(v: unknown): string {
  if (typeof v === 'number') return Number.isInteger(v) ? formatCount(v) : String(v);
  if (typeof v === 'boolean') return v ? 'yes' : 'no';
  return typeof v === 'string' ? v : JSON.stringify(v);
}

function parseObject(data: string): Record<string, unknown> | undefined {
  try {
    const v: unknown = JSON.parse(data);
    return typeof v === 'object' && v !== null && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

/** One activity event as a title and labelled facts. Nothing is dropped: an unknown key or non-JSON data is still shown. */
export function describeEvent(e: SseEvent): EventDescription {
  const title = capitalise(e.event);
  const fields = parseObject(e.data);
  if (fields === undefined) return { title, facts: e.data === '' ? [] : [['Data', valueText(e.data)]] };
  const facts = Object.entries(fields).map(([key, v]): Fact => [LABELS[key] ?? key.replace(/_/g, ' '), valueText(v)]);
  return { title, facts };
}
