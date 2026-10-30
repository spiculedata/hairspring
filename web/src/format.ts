export type Tone = 'good' | 'bad' | 'busy' | 'idle';

export function relativeTime(ms: number, now: number = Date.now()): string {
  const secs = Math.floor((now - ms) / 1000);
  if (secs < 0) return 'just now';
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86_400)}d ago`;
}

const ABSOLUTE: Intl.DateTimeFormatOptions = {
  day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23', timeZoneName: 'short',
};
const TIME_ONLY: Intl.DateTimeFormatOptions = { hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23', timeZoneName: 'short' };

/** A moment as date, time to the second and zone, in UK order ("30 Oct 2026, 15:44:50 GMT"). `timeZone` defaults to the reader's. */
export function absoluteTime(ms: number, timeZone?: string): string {
  return new Intl.DateTimeFormat('en-GB', { ...ABSOLUTE, timeZone }).format(ms);
}

/** Just the time of day and zone ("15:44:50 GMT"), for a line that says when the data was loaded. */
export function timeOfDay(ms: number, timeZone?: string): string {
  return new Intl.DateTimeFormat('en-GB', { ...TIME_ONLY, timeZone }).format(ms);
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const secs = Math.round(ms / 1000);
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

export function stateTone(state: string): Tone {
  switch (state) {
    case 'passed':
      return 'good';
    case 'failed':
    case 'error':
      return 'bad';
    case 'running':
      return 'busy';
    default:
      return 'idle';
  }
}

export type SeedParse = { ok: true; value: number | undefined } | { ok: false; message: string };

/** Seeds are u64 on the server; JavaScript can only represent integers up to 2^53 exactly. */
export function formatSeedInput(text: string): SeedParse {
  const t = text.trim();
  if (t === '') return { ok: true, value: undefined };
  if (!/^\d+$/.test(t)) return { ok: false, message: 'Seed must be a non-negative whole number' };
  const n = Number(t);
  if (!Number.isSafeInteger(n)) return { ok: false, message: 'Seed is too large for the browser; use the CLI for seeds above 2^53' };
  return { ok: true, value: n };
}
