/**
 * The seed explorer's filters: what the address can hold, what the server accepts, and the request URLs built from them.
 *
 * The rules match the server's (crates/hairspring-service/src/query.rs): a property is a catalog index of plain digits that fits
 * in 32 bits, a fingerprint is 16 to 64 hex digits, a verdict is one of five words. A value that fails its rule is
 * dropped when the address is read, so a pasted link cannot make the page send a request the server would refuse.
 */
import { runPath } from '../router';
import type { TrialVerdict } from '../trial/types';

export const VERDICT_FILTERS: readonly TrialVerdict[] = ['violation', 'inconclusive', 'unusable', 'infra', 'pass'];

const MAX_U32 = 4_294_967_295;
const MAX_IDX_DIGITS = 10;
const MAX_FROM_DIGITS = 15;
const FP = /^[0-9a-fA-F]{16,64}$/;

/** Whether `s` is a schedule fingerprint the server will accept as a filter. */
export const isFingerprint = (s: string): boolean => FP.test(s);

export interface SeedFilters {
  readonly verdict?: TrialVerdict;
  /** Only trials in which this property (catalog index) failed. */
  readonly fails?: number;
  /** Only trials that reached this property. */
  readonly hits?: number;
  /** Only usable trials that did not reach this property. */
  readonly misses?: number;
  /** Schedule fingerprint, lower case. */
  readonly fp?: string;
  /** Start the list at this sequence number. */
  readonly from?: number;
}

export type PropertyMode = 'fails' | 'hits' | 'misses';

const digits = (s: string | null, max: number): s is string => s !== null && s.length > 0 && s.length <= max && /^\d+$/.test(s);

function index(s: string | null): number | undefined {
  if (!digits(s, MAX_IDX_DIGITS)) return undefined;
  const n = Number(s);
  return n <= MAX_U32 ? n : undefined;
}

/** The filters in an address's query string. Anything invalid is left out; only one property filter is kept. */
export function filtersFromSearch(search: string): SeedFilters {
  const p = new URLSearchParams(search);
  const verdict = VERDICT_FILTERS.find((v) => v === p.get('verdict'));
  const fails = index(p.get('fails'));
  const hits = fails === undefined ? index(p.get('hits')) : undefined;
  const misses = fails === undefined && hits === undefined ? index(p.get('misses')) : undefined;
  const fpText = p.get('fp');
  const fromText = p.get('from');
  const from = digits(fromText, MAX_FROM_DIGITS) && Number.isSafeInteger(Number(fromText)) ? Number(fromText) : undefined;
  const out: SeedFilters = {
    ...(verdict !== undefined && { verdict }),
    ...(fails !== undefined && { fails }),
    ...(hits !== undefined && { hits }),
    ...(misses !== undefined && { misses }),
    ...(fpText !== null && FP.test(fpText) && { fp: fpText.toLowerCase() }),
    ...(from !== undefined && { from }),
  };
  return out;
}

function filterParams(f: SeedFilters): URLSearchParams {
  const p = new URLSearchParams();
  if (f.verdict !== undefined) p.set('verdict', f.verdict);
  if (f.fails !== undefined) p.set('fails', String(f.fails));
  if (f.hits !== undefined) p.set('hits', String(f.hits));
  if (f.misses !== undefined) p.set('misses', String(f.misses));
  if (f.fp !== undefined) p.set('fp', f.fp);
  return p;
}

/** The query string for an address, with a leading `?`, or an empty string when nothing is set. */
export function filtersToSearch(f: SeedFilters): string {
  const p = filterParams(f);
  if (f.from !== undefined) p.set('from', String(f.from));
  const text = p.toString();
  return text === '' ? '' : `?${text}`;
}

/** The explorer's path for a run, or `undefined` if the run id is not a valid one. */
export function seedsPath(runId: string, f: SeedFilters): string | undefined {
  const base = runPath(runId);
  return base === '/' ? undefined : `${base}/seeds${filtersToSearch(f)}`;
}

export const hasFilters = (f: SeedFilters): boolean => Object.values(f).some((v) => v !== undefined);

export function propertyOf(f: SeedFilters): { mode: PropertyMode; idx: number } | undefined {
  if (f.fails !== undefined) return { mode: 'fails', idx: f.fails };
  if (f.hits !== undefined) return { mode: 'hits', idx: f.hits };
  if (f.misses !== undefined) return { mode: 'misses', idx: f.misses };
  return undefined;
}

/** The filters with their property filter replaced (or removed, when `idx` is undefined). The original is not changed. */
export function withProperty(f: SeedFilters, mode: PropertyMode, idx: number | undefined): SeedFilters {
  const { fails: _fails, hits: _hits, misses: _misses, ...rest } = f;
  return idx === undefined ? rest : { ...rest, [mode]: idx };
}

export interface Page {
  readonly cursor?: string;
  readonly limit?: number;
}

/** The trials request for a set of filters. A starting point `from` becomes the cursor of the trial before it. */
export function trialsUrl(runId: string, f: SeedFilters, page: Page): string {
  const p = filterParams(f);
  const cursor = page.cursor ?? (f.from !== undefined && f.from > 0 ? String(f.from - 1) : undefined);
  if (cursor !== undefined) p.set('cursor', cursor);
  if (page.limit !== undefined) p.set('limit', String(page.limit));
  const text = p.toString();
  return `/api/runs/${encodeURIComponent(runId)}/trials${text === '' ? '' : `?${text}`}`;
}

export function stripUrl(runId: string, page: Page): string {
  const p = new URLSearchParams();
  if (page.cursor !== undefined) p.set('cursor', page.cursor);
  if (page.limit !== undefined) p.set('limit', String(page.limit));
  const text = p.toString();
  return `/api/runs/${encodeURIComponent(runId)}/strip${text === '' ? '' : `?${text}`}`;
}
