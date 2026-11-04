import { ApiError, request, requestText } from '../api';
import { isTrialDetail, isTrialPage, type StreamName, type TrialDetailResponse, type TrialRow } from './types';

const runBase = (runId: string): string => `/api/runs/${encodeURIComponent(runId)}`;

export const trialUrl = (runId: string, seq: number): string => `${runBase(runId)}/trials/${seq}`;
export const outputUrl = (runId: string, seq: number, stream: StreamName): string => `${trialUrl(runId, seq)}/output/${stream}`;
/** One trial by its exact seed. The seed is a decimal string the route has already checked. */
export const trialBySeedUrl = (runId: string, seed: string): string => `${runBase(runId)}/trials?seed=${encodeURIComponent(seed)}&limit=1`;

/** The first trial of a run that the server judged a violation. */
export const firstViolationUrl = (runId: string): string => `${runBase(runId)}/trials?verdict=violation&limit=1`;

const BAD_SHAPE = 'the server sent a trial response in an unexpected shape';

export const trialApi = {
  detail: async (runId: string, seq: number, signal?: AbortSignal): Promise<TrialDetailResponse> => {
    const body = await request<unknown>(trialUrl(runId, seq), { signal });
    if (!isTrialDetail(body)) throw new ApiError(502, BAD_SHAPE);
    return body;
  },

  /** The sequence number of the trial that ran `seed`, or undefined if the run has no such trial. */
  seqOfSeed: async (runId: string, seed: string, signal?: AbortSignal): Promise<number | undefined> => {
    const body = await request<unknown>(trialBySeedUrl(runId, seed), { signal });
    if (!isTrialPage(body)) throw new ApiError(502, BAD_SHAPE);
    return body.trials[0]?.seq;
  },

  /** The first violating trial of a run, or undefined if the run kept none. */
  firstViolation: async (runId: string, signal?: AbortSignal): Promise<TrialRow | undefined> => {
    const body = await request<unknown>(firstViolationUrl(runId), { signal });
    if (!isTrialPage(body)) throw new ApiError(502, BAD_SHAPE);
    return body.trials[0];
  },

  output: (runId: string, seq: number, stream: StreamName, signal?: AbortSignal): Promise<string> =>
    requestText(outputUrl(runId, seq, stream), { signal }),
};
