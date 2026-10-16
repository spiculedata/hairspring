import { useEffect, useState } from 'react';

const NAV_EVENT = 'hairspring:navigate';

export function navigate(path: string): void {
  if (window.location.pathname !== path) window.history.pushState({}, '', path);
  window.dispatchEvent(new Event(NAV_EVENT));
}

export function usePath(): string {
  const [path, setPath] = useState(window.location.pathname);
  useEffect(() => {
    const update = () => setPath(window.location.pathname);
    window.addEventListener('popstate', update);
    window.addEventListener(NAV_EVENT, update);
    return () => {
      window.removeEventListener('popstate', update);
      window.removeEventListener(NAV_EVENT, update);
    };
  }, []);
  return path;
}

const MAX_SEED_DIGITS = 20;
const MAX_SEQ_DIGITS = 19;
const RUN_ID = /^[A-Za-z0-9_-]{1,64}$/;

const isDigits = (s: string, max: number): boolean => s.length > 0 && s.length <= max && /^\d+$/.test(s);

/** A run's page, or the run list if the id is not valid. */
export function runPath(runId: string): string {
  return RUN_ID.test(runId) ? `/runs/${runId}` : '/';
}

/** The failure page of a run's trial that ran a seed (a decimal string). Undefined if either piece is not valid. */
export function seedPath(runId: string, seed: string): string | undefined {
  return RUN_ID.test(runId) && isDigits(seed, MAX_SEED_DIGITS) ? `/runs/${runId}/seeds/${seed}` : undefined;
}

/** The failure page of a run's trial by its sequence number. */
export function trialPath(runId: string, seq: number): string | undefined {
  return RUN_ID.test(runId) && Number.isSafeInteger(seq) && seq >= 0 ? `/runs/${runId}/trials/${seq}` : undefined;
}

export type Route =
  | { name: 'runs' }
  | { name: 'run'; id: string }
  | { name: 'trial'; id: string; seq: string }
  | { name: 'seed'; id: string; seed: string }
  | { name: 'failure'; id: string }
  | { name: 'targets' }
  | { name: 'workers' }
  | { name: 'notfound' };

export function matchRoute(path: string): Route {
  const parts = path.split('/').filter(Boolean);
  if (parts.length === 0 || (parts.length === 1 && parts[0] === 'runs')) return { name: 'runs' };
  if (parts.length === 2 && parts[0] === 'runs') return { name: 'run', id: parts[1] };
  if (parts.length === 4 && parts[0] === 'runs' && parts[2] === 'trials' && isDigits(parts[3], MAX_SEQ_DIGITS)) {
    return { name: 'trial', id: parts[1], seq: parts[3] };
  }
  if (parts.length === 4 && parts[0] === 'runs' && parts[2] === 'seeds' && isDigits(parts[3], MAX_SEED_DIGITS)) {
    return { name: 'seed', id: parts[1], seed: parts[3] };
  }
  if (parts.length === 2 && parts[0] === 'failures') return { name: 'failure', id: parts[1] };
  if (parts.length === 1 && parts[0] === 'targets') return { name: 'targets' };
  if (parts.length === 1 && parts[0] === 'workers') return { name: 'workers' };
  return { name: 'notfound' };
}
