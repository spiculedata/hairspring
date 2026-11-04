/** How many failed requests in a row it takes before the page says the server cannot be reached. One may be a blip. */
export const OFFLINE_AFTER_FAILURES = 2;

export interface Connection {
  /** The most requests that any one poll on the page has failed in a row; a poll's success or end clears its own count. */
  readonly failures: number;
}

/** What one poll reports through. Each poll registers its own, so it can only clear its own failures. */
export interface PollerStatus {
  success: () => void;
  failure: () => void;
  /** Call when the poll ends for any reason; its failures stop counting. */
  unregister: () => void;
}

const NONE: Connection = { failures: 0 };
let current: Connection = NONE;
const pollers = new Map<symbol, number>();
const listeners = new Set<() => void>();

function recompute(): void {
  const failures = Math.max(0, ...pollers.values());
  if (failures === current.failures) return;
  current = failures === 0 ? NONE : { failures };
  for (const listener of listeners) listener();
}

export const getConnection = (): Connection => current;

export function subscribeConnection(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function registerPoller(): PollerStatus {
  const id = Symbol('poller');
  pollers.set(id, 0);
  const update = (change: (failures: number) => number) => {
    const failures = pollers.get(id);
    if (failures === undefined) return;
    pollers.set(id, change(failures));
    recompute();
  };
  return {
    success: () => update(() => 0),
    failure: () => update((failures) => failures + 1),
    unregister: () => {
      pollers.delete(id);
      recompute();
    },
  };
}

/** For tests: forget every poller and everything reported so far. */
export function resetConnection(): void {
  pollers.clear();
  recompute();
}
