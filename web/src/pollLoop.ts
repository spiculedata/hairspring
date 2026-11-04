import { reportPollFailure, reportPollSuccess } from './connectionStatus';
import { backoffDelay, isConnectivityFailure, retryDelay, STOP_ON_UNAUTHORIZED } from './pollPolicy';

export interface PollOptions<T> {
  load: () => Promise<T>;
  intervalMs: number;
  /** Returns true when `data` is final and polling can end. */
  stop?: (data: T) => boolean;
  onData: (data: T) => void;
  onError: (error: unknown) => void;
}

const hasBrowser = typeof document !== 'undefined' && typeof window !== 'undefined';
const isHidden = (): boolean => hasBrowser && document.visibilityState === 'hidden';

/**
 * Loads now and then every `intervalMs` until `stop(data)` is true. A failure is reported and retried by the rules in
 * pollPolicy.ts: a 404 or a 401 ends the loop (a 401 has raised the token prompt once; the caller starts a new loop when
 * a token is entered), a 429 waits at least as long as Retry-After, and failures in a row wait longer each time.
 *
 * While the tab is hidden no request is made; when it is shown again the loop refreshes at once. The browser coming back
 * online also refreshes at once. Results are reported to connectionStatus.ts so one banner can say the server is out of
 * reach. Returns a function that cancels the loop; nothing is delivered after that.
 */
export function startPolling<T>({ load, intervalMs, stop, onData, onError }: PollOptions<T>): () => void {
  let ended = false;
  let inFlight = false;
  let failures = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const clearTimer = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const schedule = (delay: number) => {
    clearTimer();
    if (!isHidden()) timer = setTimeout(run, delay);
  };
  const end = () => {
    ended = true;
    clearTimer();
    if (!hasBrowser) return;
    document.removeEventListener('visibilitychange', onVisibility);
    window.removeEventListener('online', onOnline);
  };
  const refreshNow = () => {
    if (ended || inFlight) return;
    clearTimer();
    void run();
  };
  const onVisibility = () => (isHidden() ? clearTimer() : refreshNow());
  const onOnline = () => {
    if (isHidden()) return;
    failures = 0;
    refreshNow();
  };

  async function run(): Promise<void> {
    timer = undefined;
    if (ended || inFlight) return;
    inFlight = true;
    let delay = intervalMs;
    try {
      const next = await load();
      if (ended) return;
      failures = 0;
      reportPollSuccess();
      onData(next);
      if (stop?.(next)) return end();
    } catch (e) {
      if (ended) return;
      if (isConnectivityFailure(e)) {
        failures += 1;
        reportPollFailure();
      } else {
        failures = 0;
        reportPollSuccess();
      }
      onError(e);
      const wait = retryDelay(e, intervalMs, STOP_ON_UNAUTHORIZED);
      if (wait === undefined) return end();
      delay = Math.max(wait, backoffDelay(intervalMs, failures));
    } finally {
      inFlight = false;
    }
    schedule(delay);
  }

  if (hasBrowser) {
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('online', onOnline);
  }
  void run();
  return end;
}
