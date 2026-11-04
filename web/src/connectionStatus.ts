/** How many failed requests in a row it takes before the page says the server cannot be reached. One may be a blip. */
export const OFFLINE_AFTER_FAILURES = 2;

export interface Connection {
  /** Requests that have failed in a row, across every poll on the page; any success clears it. */
  readonly failures: number;
}

const NONE: Connection = { failures: 0 };
let current: Connection = NONE;
const listeners = new Set<() => void>();

function set(next: Connection): void {
  if (next.failures === current.failures) return;
  current = next;
  for (const listener of listeners) listener();
}

export const getConnection = (): Connection => current;

export function subscribeConnection(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export const reportPollSuccess = (): void => set(NONE);
export const reportPollFailure = (): void => set({ failures: current.failures + 1 });

/** For tests: forget everything reported so far. */
export const resetConnection = (): void => set(NONE);
