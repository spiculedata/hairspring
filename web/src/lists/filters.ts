/**
 * The filters of the Runs and Failures lists: what the address can hold, what the server accepts, and the request
 * parameters built from them.
 *
 * The rules match the server's (crates/hairspring-service/src/list_pages.rs). A value that fails its rule is dropped when
 * the address is read, and refused when it is typed, so a pasted link or a typo cannot make the page send a request the
 * server would answer with a 400.
 */
import { FINDING_STATUSES, type FindingStatus, type RunState } from '../types';

export const RUN_STATE_FILTERS: readonly RunState[] = ['queued', 'running', 'passed', 'failed', 'error', 'cancelled'];
export const RUN_MODE_FILTERS = ['search', 'sweep'] as const;
export type RunModeFilter = (typeof RUN_MODE_FILTERS)[number];

/** Longest target name, in characters, and longest property id prefix, in bytes. */
export const MAX_TARGET_CHARS = 64;
export const MAX_PROPERTY_PREFIX_BYTES = 128;

const RUN_ID = /^run_[0-9a-fA-F]{16}$/;
/** Control characters; the server refuses them in a target or a prefix. */
const CONTROL = /\p{Cc}/u;

export interface RunFilters {
  readonly state?: RunState;
  readonly target?: string;
  readonly mode?: RunModeFilter;
}

export interface FailureFilters {
  /** A run id: only that run's failures. */
  readonly run?: string;
  /** A property id prefix. */
  readonly property?: string;
  readonly status?: FindingStatus;
}

export type FilterProblem = 'target' | 'property' | 'run';

const byteLength = (s: string): number => new TextEncoder().encode(s).length;

/** The target name to filter by, or why it cannot be used. An empty text means no filter. */
export function checkTarget(text: string): { value: string | undefined } | { problem: FilterProblem } {
  if (text === '') return { value: undefined };
  return [...text].length <= MAX_TARGET_CHARS && !CONTROL.test(text) ? { value: text } : { problem: 'target' };
}

/** The property id prefix to filter by, or why it cannot be used. An empty text means no filter. */
export function checkPropertyPrefix(text: string): { value: string | undefined } | { problem: FilterProblem } {
  if (text === '') return { value: undefined };
  return byteLength(text) <= MAX_PROPERTY_PREFIX_BYTES && !CONTROL.test(text) ? { value: text } : { problem: 'property' };
}

/** The run id to filter by, or why it cannot be used. An empty text means no filter. */
export function checkRunId(text: string): { value: string | undefined } | { problem: FilterProblem } {
  if (text === '') return { value: undefined };
  return RUN_ID.test(text) ? { value: text } : { problem: 'run' };
}

function textOf(p: URLSearchParams, key: string, check: (text: string) => { value: string | undefined } | { problem: FilterProblem }): string | undefined {
  const text = p.get(key);
  if (text === null) return undefined;
  const checked = check(text);
  return 'value' in checked ? checked.value : undefined;
}

/** The run filters in an address's query string. Anything invalid is left out. */
export function runFiltersFromSearch(search: string): RunFilters {
  const p = new URLSearchParams(search);
  const state = RUN_STATE_FILTERS.find((s) => s === p.get('state'));
  const mode = RUN_MODE_FILTERS.find((m) => m === p.get('mode'));
  const target = textOf(p, 'target', checkTarget);
  return { ...(state !== undefined && { state }), ...(target !== undefined && { target }), ...(mode !== undefined && { mode }) };
}

/** The failure filters in an address's query string. Anything invalid is left out. */
export function failureFiltersFromSearch(search: string): FailureFilters {
  const p = new URLSearchParams(search);
  const status = FINDING_STATUSES.find((s) => s === p.get('status'));
  const run = textOf(p, 'run', checkRunId);
  const property = textOf(p, 'property', checkPropertyPrefix);
  return { ...(run !== undefined && { run }), ...(property !== undefined && { property }), ...(status !== undefined && { status }) };
}

const searchOf = (p: URLSearchParams): string => (p.size === 0 ? '' : `?${p.toString()}`);

/** The query string for an address, with a leading `?`, or an empty string when nothing is set. */
export function runFiltersToSearch(f: RunFilters): string {
  const p = new URLSearchParams();
  if (f.state !== undefined) p.set('state', f.state);
  if (f.target !== undefined) p.set('target', f.target);
  if (f.mode !== undefined) p.set('mode', f.mode);
  return searchOf(p);
}

export function failureFiltersToSearch(f: FailureFilters): string {
  const p = new URLSearchParams();
  if (f.run !== undefined) p.set('run', f.run);
  if (f.property !== undefined) p.set('property', f.property);
  if (f.status !== undefined) p.set('status', f.status);
  return searchOf(p);
}

export const hasRunFilters = (f: RunFilters): boolean => f.state !== undefined || f.target !== undefined || f.mode !== undefined;
export const hasFailureFilters = (f: FailureFilters): boolean => f.run !== undefined || f.property !== undefined || f.status !== undefined;
