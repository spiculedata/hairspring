import { ApiError, request } from '../api';
import type { TrialRow } from '../trial/types';
import type { PropertyDetail } from '../types';
import { stripUrl, trialsUrl, type Page, type SeedFilters } from './filters';

/** One page of `GET /api/runs/{id}/strip`: a code per trial from `first_seq`, in judging order. */
export interface StripPage {
  run_id: string;
  first_seq: number;
  codes: string;
  legend: ReadonlyArray<{ code: string; name: string }>;
  next_cursor: string | null;
  scanned_trials: number;
}

/** One page of `GET /api/runs/{id}/trials`. `next_cursor` is the last trial read, which may be past the last row shown. */
export interface SeedTrialPage {
  run_id: string;
  trials: readonly TrialRow[];
  next_cursor: string | null;
  scanned_trials: number;
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isCursor = (v: unknown): boolean => v === null || (typeof v === 'string' && /^\d{1,20}$/.test(v));

export function isStripPage(v: unknown): v is StripPage {
  return isObject(v) && typeof v.first_seq === 'number' && typeof v.codes === 'string' && isCursor(v.next_cursor);
}

function isRow(v: unknown): boolean {
  if (!isObject(v) || !isObject(v.guest)) return false;
  return (
    typeof v.seq === 'number' &&
    typeof v.seed === 'string' &&
    typeof v.verdict === 'string' &&
    typeof v.class === 'string' &&
    typeof v.failing_count === 'number' &&
    Array.isArray(v.guest.failing_ids)
  );
}

export function isSeedTrialPage(v: unknown): v is SeedTrialPage {
  return isObject(v) && Array.isArray(v.trials) && v.trials.every(isRow) && isCursor(v.next_cursor) && typeof v.scanned_trials === 'number';
}

function isPropertyDetail(v: unknown): v is PropertyDetail {
  return isObject(v) && isObject(v.property) && typeof v.property.idx === 'number' && isObject(v.property.guest) && typeof v.property.guest.id === 'string';
}

const BAD_SHAPE = (what: string): ApiError => new ApiError(502, `the server sent a ${what} response in an unexpected shape`);

export const seedsApi = {
  strip: async (runId: string, page: Page, signal?: AbortSignal): Promise<StripPage> => {
    const body = await request<unknown>(stripUrl(runId, page), { signal });
    if (!isStripPage(body)) throw BAD_SHAPE('strip');
    return body;
  },

  trials: async (runId: string, filters: SeedFilters, page: Page, signal?: AbortSignal): Promise<SeedTrialPage> => {
    const body = await request<unknown>(trialsUrl(runId, filters, page), { signal });
    if (!isSeedTrialPage(body)) throw BAD_SHAPE('trials');
    return body;
  },

  /** One property alone, with no seed list, so naming a filter costs no scan. */
  property: async (runId: string, idx: number, signal?: AbortSignal): Promise<PropertyDetail> => {
    const body = await request<unknown>(`/api/runs/${encodeURIComponent(runId)}/properties/${idx}?seeds=none`, { signal });
    if (!isPropertyDetail(body)) throw BAD_SHAPE('property');
    return body;
  },
};
