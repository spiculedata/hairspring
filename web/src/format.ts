export type Tone = 'good' | 'bad' | 'busy' | 'idle';

export function relativeTime(ms: number, now: number = Date.now()): string {
  const secs = Math.floor((now - ms) / 1000);
  if (secs < 0) return 'just now';
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86_400)}d ago`;
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
