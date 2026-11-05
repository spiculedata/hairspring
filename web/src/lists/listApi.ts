import { ApiError, request } from '../api';
import type { FailureSummary, FindingStatus, Run, RunState } from '../types';
import type { ListPage } from './listPaging';

export interface RunsQuery {
  readonly state?: RunState;
  readonly target?: string;
  readonly mode?: string;
  /** The `next_cursor` of the previous page. */
  readonly cursor?: string;
  readonly limit?: number;
}

export interface FailuresQuery {
  readonly run?: string;
  readonly property?: string;
  readonly status?: FindingStatus;
  readonly cursor?: string;
  readonly limit?: number;
}

function withQuery(path: string, entries: ReadonlyArray<readonly [string, string | number | undefined]>): string {
  const params = new URLSearchParams();
  for (const [key, value] of entries) if (value !== undefined) params.set(key, String(value));
  const qs = params.toString();
  return qs === '' ? path : `${path}?${qs}`;
}

/** The runs URL: each value is encoded, so a target cannot add or override a parameter. */
export function runsUrl(q: RunsQuery): string {
  return withQuery('/api/runs', [['state', q.state], ['target', q.target], ['mode', q.mode], ['cursor', q.cursor], ['limit', q.limit]]);
}

export function failuresUrl(q: FailuresQuery): string {
  return withQuery('/api/failures', [['run', q.run], ['property', q.property], ['status', q.status], ['cursor', q.cursor], ['limit', q.limit]]);
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isRun = (v: unknown): v is Run => isObject(v) && typeof v.id === 'string' && typeof v.state === 'string' && typeof v.target === 'string';
const isFailure = (v: unknown): v is FailureSummary => isObject(v) && typeof v.id === 'string' && typeof v.run_id === 'string' && typeof v.created_at === 'number';

/** A list body as a page, or an error when it is not the shape the page reads, so a wrong body is shown as one and never crashes the table. */
function pageOf<T>(body: unknown, key: string, isItem: (v: unknown) => v is T, what: string): ListPage<T> {
  const items = isObject(body) ? body[key] : undefined;
  const cursor = isObject(body) ? body.next_cursor : undefined;
  if (!Array.isArray(items) || !items.every(isItem) || (cursor !== undefined && typeof cursor !== 'string')) {
    throw new ApiError(502, `the server sent the ${what} in an unexpected shape`);
  }
  return { items, nextCursor: cursor };
}

export async function runsPage(q: RunsQuery, signal?: AbortSignal): Promise<ListPage<Run>> {
  return pageOf(await request<unknown>(runsUrl(q), { signal }), 'runs', isRun, 'list of runs');
}

export async function failuresPage(q: FailuresQuery, signal?: AbortSignal): Promise<ListPage<FailureSummary>> {
  return pageOf(await request<unknown>(failuresUrl(q), { signal }), 'failures', isFailure, 'list of failures');
}
