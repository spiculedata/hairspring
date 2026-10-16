import { useEffect, useState } from 'react';
import { api } from '../api';
import { usePolling, type Polled } from '../usePolling';
import { trialApi } from './api';
import type { TailState } from './OutputTail';
import type { RunContext } from './TrialHeader';
import type { StreamName, TrialDetailResponse } from './types';

/** A kept trial never changes, so it is loaded once; this interval only keeps a failed load from retrying at once. */
const RETRY_MS = 30_000;
const REVOKE_DELAY_MS = 10_000;

export function useTrialDetail(runId: string, seq: number): Polled<TrialDetailResponse> {
  return usePolling(() => trialApi.detail(runId, seq), RETRY_MS, () => true);
}

/**
 * What the run says about how it ended, from the first row of its property page. The failure page is complete without
 * it, so a failed load shows nothing extra and never an error.
 */
export function useRunContext(runId: string): RunContext | undefined {
  const [context, setContext] = useState<RunContext>();
  useEffect(() => {
    const ctl = new AbortController();
    api
      .properties(runId, { limit: 1 }, ctl.signal)
      .then((page) => {
        if (!ctl.signal.aborted) {
          setContext({ stoppedAtFirstFailure: page.header.stopped_at_first_failure, completionLabel: page.header.completion_label });
        }
      })
      .catch(() => undefined);
    return () => ctl.abort();
  }, [runId]);
  return context;
}

type Tails = Partial<Record<StreamName, TailState>>;

/** Loads each kept stream of a trial once, keeping a failure of one stream from hiding the other. */
export function useOutputTails(runId: string, seq: number, streams: readonly StreamName[]): Tails {
  const [tails, setTails] = useState<Tails>({});
  const key = streams.join(',');
  useEffect(() => {
    const ctl = new AbortController();
    setTails(Object.fromEntries(streams.map((s) => [s, { status: 'loading' } as TailState])));
    for (const stream of streams) {
      trialApi
        .output(runId, seq, stream, ctl.signal)
        .then((text) => {
          if (!ctl.signal.aborted) setTails((prev) => ({ ...prev, [stream]: { status: 'ready', text } }));
        })
        .catch((e: unknown) => {
          if (ctl.signal.aborted) return;
          const error = e instanceof Error ? e.message : String(e);
          setTails((prev) => ({ ...prev, [stream]: { status: 'error', error } }));
        });
    }
    return () => ctl.abort();
    // `key` stands in for `streams`, which is a new array on every render.
  }, [runId, seq, key]);
  return tails;
}

/** Saves a stream's whole kept tail as a text file. The file name is built from the run, sequence and stream only. */
export async function downloadOutput(runId: string, seq: number, stream: StreamName): Promise<void> {
  const text = await trialApi.output(runId, seq, stream);
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = `${runId}-${seq}-${stream}.txt`;
  a.click();
  // Revoking at once can cancel the download in some browsers.
  setTimeout(() => URL.revokeObjectURL(url), REVOKE_DELAY_MS);
}
