import { parseSse, type SseEvent } from './sse';
import type { FailureDetail, FailureSummary, PropertyPage, PropertyQuery, ReplayResult, Run, RunDetail, Target, Worker } from './types';

const TOKEN_KEY = 'hairspring.token';

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
    /** How long the server asked callers to wait (429 Retry-After), in milliseconds. */
    public retryAfterMs?: number,
  ) {
    super(message);
  }
}

const MAX_RETRY_AFTER_MS = 300_000;

/** A `Retry-After` header (delta-seconds or an HTTP date) in milliseconds, capped; `undefined` if absent or invalid. */
export function parseRetryAfter(value: string | null, now: number): number | undefined {
  if (value === null) return undefined;
  const text = value.trim();
  const isDelta = /^\d+$/.test(text);
  if (!isDelta && !/[a-z]/i.test(text)) return undefined;
  const ms = isDelta ? Number(text) * 1000 : Date.parse(text) - now;
  if (!Number.isFinite(ms) || ms < 0) return undefined;
  return Math.min(ms, MAX_RETRY_AFTER_MS);
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** Checks the parts of a properties response the page reads, so a wrong-shaped body is an error and not a crash. */
export function isPropertyPage(v: unknown): v is PropertyPage {
  if (!isObject(v)) return false;
  if (typeof v.headline !== 'string' || !isObject(v.header) || !isObject(v.claim) || !isObject(v.counts_by_status)) return false;
  if (!Array.isArray(v.notes) || typeof v.total !== 'number' || !Array.isArray(v.properties)) return false;
  return v.properties.every((p) => isObject(p) && isObject(p.guest) && isObject(p.rate) && typeof p.status === 'string');
}

type Listener = () => void;
const unauthorizedListeners = new Set<Listener>();

/** Called whenever the server answers 401 so the UI can ask for a token. */
export function onUnauthorized(fn: Listener): () => void {
  unauthorizedListeners.add(fn);
  return () => unauthorizedListeners.delete(fn);
}

export function getToken(): string | null {
  try {
    return sessionStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string | null): void {
  try {
    if (token) sessionStorage.setItem(TOKEN_KEY, token);
    else sessionStorage.removeItem(TOKEN_KEY);
  } catch {
    // Storage can be unavailable (private mode); the token then lives only until reload.
  }
}

function headers(json: boolean): Record<string, string> {
  const h: Record<string, string> = {};
  const token = getToken();
  if (token) h.Authorization = `Bearer ${token}`;
  if (json) h['Content-Type'] = 'application/json';
  return h;
}

async function failure(res: Response): Promise<ApiError> {
  if (res.status === 401) unauthorizedListeners.forEach((fn) => fn());
  let message = res.statusText || `HTTP ${res.status}`;
  try {
    const body = await res.json();
    if (typeof body?.error === 'string') message = body.error;
  } catch {
    // Non-JSON error body: keep the status text.
  }
  return new ApiError(res.status, message, parseRetryAfter(res.headers.get('Retry-After'), Date.now()));
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(path, { ...init, headers: { ...headers(init.body !== undefined), ...(init.headers as Record<string, string>) } });
  if (!res.ok) throw await failure(res);
  return (await res.json()) as T;
}

/** The properties URL for a run: each filter is encoded, so a prefix cannot add or override a parameter. */
export function propertiesUrl(runId: string, query: PropertyQuery): string {
  const params = new URLSearchParams();
  if (query.status) params.set('status', query.status);
  if (query.kind) params.set('kind', query.kind);
  if (query.q) params.set('q', query.q);
  if (query.sort) params.set('sort', query.sort);
  if (query.cursor) params.set('cursor', query.cursor);
  if (query.limit !== undefined) params.set('limit', String(query.limit));
  const qs = params.toString();
  return `/api/runs/${encodeURIComponent(runId)}/properties${qs ? `?${qs}` : ''}`;
}

export const api = {
  properties: async (runId: string, query: PropertyQuery = {}, signal?: AbortSignal): Promise<PropertyPage> => {
    const body = await request<unknown>(propertiesUrl(runId, query), { signal });
    if (!isPropertyPage(body)) throw new ApiError(502, 'the server sent a properties response in an unexpected shape');
    return body;
  },
  health: () => request<{ status: string; version: string }>('/api/health'),
  targets: async () => (await request<{ targets: Target[] }>('/api/targets')).targets,
  runs: async (limit = 50) => (await request<{ runs: Run[] }>(`/api/runs?limit=${limit}`)).runs,
  run: (id: string) => request<RunDetail>(`/api/runs/${encodeURIComponent(id)}`),
  startRun: (target: string, seed?: number) =>
    request<{ run: Run }>('/api/runs', { method: 'POST', body: JSON.stringify({ target, seed }) }),
  cancel: (id: string) => request<{ cancelled: boolean }>(`/api/runs/${encodeURIComponent(id)}/cancel`, { method: 'POST', body: '{}' }),
  failures: async (limit = 50) => (await request<{ failures: FailureSummary[] }>(`/api/failures?limit=${limit}`)).failures,
  failure: (id: string) => request<FailureDetail>(`/api/failures/${encodeURIComponent(id)}`),
  replay: (bundle: string) => request<ReplayResult>('/api/replay', { method: 'POST', body: JSON.stringify({ bundle }) }),
  workers: async () => (await request<{ workers: Worker[] }>('/api/workers')).workers,
};

/** Streams run events with fetch (EventSource cannot send an Authorization header). */
export async function streamRunEvents(runId: string, onEvent: (e: SseEvent) => void, signal: AbortSignal): Promise<void> {
  const res = await fetch(`/api/runs/${encodeURIComponent(runId)}/events`, { headers: headers(false), signal });
  if (!res.ok || !res.body) throw await failure(res);
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buffer += decoder.decode(value, { stream: true });
    const { events, rest } = parseSse(buffer);
    buffer = rest;
    events.forEach(onEvent);
  }
}
