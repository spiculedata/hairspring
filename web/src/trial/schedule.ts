/**
 * Reading the scheduler summary's text fields. The worker reports them as JSON text, and none of it is trusted: a field
 * that is not in the expected shape is left for the page to show as the text it was, never guessed at.
 */

export interface TimesliceStats {
  readonly count: number;
  readonly sumNs: number;
}

export interface ThreadSlices {
  readonly columns: readonly string[];
  /** One row per entry, each cell the text of a scalar value. */
  readonly rows: ReadonlyArray<readonly string[]>;
}

/** Rows and columns read from per-thread timeslice text; more are left to the raw text. */
export const MAX_SLICE_ROWS = 200;
export const MAX_SLICE_COLUMNS = 8;

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return undefined;
  }
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isSafeInteger(v) && v >= 0;
const isScalar = (v: unknown): v is string | number | boolean => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean';

/** `{"count":3,"sum_ns":1500}` as a count and a total, or undefined if the text is anything else. */
export function parseTimesliceStats(text: string): TimesliceStats | undefined {
  const v = parseJson(text);
  if (!isObject(v) || !isCount(v.count) || !isCount(v.sum_ns)) return undefined;
  return { count: v.count, sumNs: v.sum_ns };
}

/** An array of flat objects as a table, or undefined when it is empty, nested, too wide or too long to read as one. */
export function parseThreadSlices(text: string): ThreadSlices | undefined {
  const v = parseJson(text);
  if (!Array.isArray(v) || v.length === 0 || v.length > MAX_SLICE_ROWS) return undefined;
  if (!v.every((row): row is Record<string, string | number | boolean> => isObject(row) && Object.values(row).every(isScalar))) return undefined;
  const columns = [...new Set(v.flatMap((row) => Object.keys(row)))];
  if (columns.length === 0 || columns.length > MAX_SLICE_COLUMNS) return undefined;
  return { columns, rows: v.map((row) => columns.map((c) => (c in row ? String(row[c]) : ''))) };
}
