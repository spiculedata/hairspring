import { ApiError, request } from '../api';

export interface RunHit {
  readonly id: string;
  readonly target: string;
  readonly state: string;
}
export interface FailureHit {
  readonly id: string;
  readonly run_id: string;
  readonly target: string;
}
export interface TargetHit {
  readonly name: string;
}
/** A property id from the program under test: untrusted text, shown only through `GuestText`. */
export interface PropertyHit {
  readonly run_id: string;
  readonly guest: { readonly note: string; readonly id: string };
}
/** `trial` is null when the seed is the one the run was started with, and a trial's number when a trial ran it. */
export interface SeedHit {
  readonly run_id: string;
  readonly seed: number;
  readonly trial: number | null;
}
export interface ScheduleHit {
  readonly fingerprint: string;
  readonly run_id: string;
  readonly trial: number;
}

/** The answer of `GET /api/search`: groups of at most `scope.per_group` hits, `more` naming the groups that have others. */
export interface SearchResults {
  readonly runs: readonly RunHit[];
  readonly failures: readonly FailureHit[];
  readonly targets: readonly TargetHit[];
  readonly properties: readonly PropertyHit[];
  readonly seeds: readonly SeedHit[];
  readonly schedules: readonly ScheduleHit[];
  readonly more: readonly string[];
  readonly scope: { readonly per_group: number; readonly recent_runs: number; readonly property_runs: number };
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isText = (v: unknown): v is string => typeof v === 'string';
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const listOf = (v: unknown, item: (x: unknown) => boolean): boolean => Array.isArray(v) && v.every(item);

const GROUP_CHECKS: Readonly<Record<string, (x: unknown) => boolean>> = {
  runs: (x) => isObject(x) && isText(x.id) && isText(x.target) && isText(x.state),
  failures: (x) => isObject(x) && isText(x.id) && isText(x.run_id) && isText(x.target),
  targets: (x) => isObject(x) && isText(x.name),
  properties: (x) => isObject(x) && isText(x.run_id) && isObject(x.guest) && isText(x.guest.id) && isText(x.guest.note),
  seeds: (x) => isObject(x) && isText(x.run_id) && isCount(x.seed) && (x.trial === null || isCount(x.trial)),
  schedules: (x) => isObject(x) && isText(x.fingerprint) && isText(x.run_id) && isCount(x.trial),
};

/** Checks the parts of a search answer the box reads, so a wrong-shaped body is an error and not a crash. */
export function isSearchResults(v: unknown): v is SearchResults {
  if (!isObject(v)) return false;
  if (!Object.entries(GROUP_CHECKS).every(([group, check]) => listOf(v[group], check))) return false;
  if (!listOf(v.more, isText)) return false;
  return isObject(v.scope) && isCount(v.scope.per_group) && isCount(v.scope.recent_runs) && isCount(v.scope.property_runs);
}

/** The search URL: the text is encoded, so it cannot add or override a parameter. */
export function searchUrl(text: string): string {
  return `/api/search?q=${encodeURIComponent(text)}`;
}

export async function searchRequest(text: string, signal: AbortSignal | undefined): Promise<SearchResults> {
  const body = await request<unknown>(searchUrl(text), { signal });
  if (!isSearchResults(body)) throw new ApiError(502, 'the server sent a search answer in an unexpected shape');
  return body;
}

/**
 * Whether this server has a search endpoint. It asks with a text the server refuses before it looks anything up (a
 * lone space), so the question costs no query. Only a 404 means the endpoint is absent (an older server); any other
 * answer, and a network failure, keep the box, since the endpoint may simply need a token or a moment.
 */
export async function probeSearchAvailable(): Promise<boolean> {
  try {
    await request<unknown>(searchUrl(' '));
    return true;
  } catch (e) {
    return !(e instanceof ApiError && e.status === 404);
  }
}
