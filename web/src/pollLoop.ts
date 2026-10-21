import { retryDelay, STOP_ON_UNAUTHORIZED } from './pollPolicy';

export interface PollOptions<T> {
  load: () => Promise<T>;
  intervalMs: number;
  /** Returns true when `data` is final and polling can end. */
  stop?: (data: T) => boolean;
  onData: (data: T) => void;
  onError: (error: unknown) => void;
}

/**
 * Loads now and then every `intervalMs` until `stop(data)` is true. A failure is reported and retried by the rules in
 * pollPolicy.ts: a 404 or a 401 ends the loop (a 401 has raised the token prompt once; the caller starts a new loop when
 * a token is entered), a 429 waits at least as long as Retry-After. Returns a function that cancels the loop; nothing is
 * delivered after that.
 */
export function startPolling<T>({ load, intervalMs, stop, onData, onError }: PollOptions<T>): () => void {
  let cancelled = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const run = async () => {
    let delay = intervalMs;
    try {
      const next = await load();
      if (cancelled) return;
      onData(next);
      if (stop?.(next)) return;
    } catch (e) {
      if (cancelled) return;
      onError(e);
      const wait = retryDelay(e, intervalMs, STOP_ON_UNAUTHORIZED);
      if (wait === undefined) return;
      delay = wait;
    }
    timer = setTimeout(run, delay);
  };
  void run();
  return () => {
    cancelled = true;
    if (timer) clearTimeout(timer);
  };
}
