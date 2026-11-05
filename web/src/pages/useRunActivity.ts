import { useEffect, useState } from 'react';
import { streamRunEvents } from '../api';
import type { SseEvent } from '../sse';

const MAX_LOG_LINES = 200;

/**
 * The run's events as they arrive, the latest few hundred. It lives with the page, not the Activity tab, so lines that
 * arrive while another tab is showing are there when the reader opens it. `active` changes when the run ends, which
 * opens the stream again so the final events are read.
 */
export function useRunActivity(runId: string, active: boolean): readonly SseEvent[] {
  const [lines, setLines] = useState<readonly SseEvent[]>([]);
  useEffect(() => {
    const ctl = new AbortController();
    streamRunEvents(runId, (e) => setLines((prev) => [...prev, e].slice(-MAX_LOG_LINES)), ctl.signal).catch(() => {
      // Aborted on navigation, or the run ended: the polled state remains authoritative.
    });
    return () => ctl.abort();
  }, [runId, active]);
  return lines;
}
