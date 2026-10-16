import type { AssertKind, PropertyQuery, PropertySort, PropertyStatus } from '../types';
import { statusLabel } from '../wording';
import { kindLabel } from './PropertyRowView';

const STATUSES: readonly PropertyStatus[] = ['violation', 'unmet', 'unrated', 'pass'];
const KINDS: readonly AssertKind[] = ['always', 'always_or_unreachable', 'sometimes', 'reachable', 'unreachable'];
const SORTS: ReadonlyArray<readonly [PropertySort, string]> = [
  ['status', 'Status'],
  ['id', 'Id'],
  ['rate', 'Rate'],
  ['evals', 'Evaluations'],
];
/** The longest id prefix the server accepts is bounded; stop typing well short of it. */
const MAX_PREFIX_CHARS = 128;

interface Props {
  query: PropertyQuery;
  onQuery: (next: PropertyQuery) => void;
}

/** Narrow a value from a `<select>` to one of the known options, or `undefined` for "all". */
function pick<T extends string>(value: string, options: readonly T[]): T | undefined {
  return options.find((o) => o === value);
}

export function PropertyFilters({ query, onQuery }: Props) {
  return (
    <div className="row filters">
      <label>
        Status
        <select value={query.status ?? ''} onChange={(e) => onQuery({ ...query, status: pick(e.target.value, STATUSES) })}>
          <option value="">All</option>
          {STATUSES.map((s) => (
            <option key={s} value={s}>
              {statusLabel(s)}
            </option>
          ))}
        </select>
      </label>
      <label>
        Kind
        <select value={query.kind ?? ''} onChange={(e) => onQuery({ ...query, kind: pick(e.target.value, KINDS) })}>
          <option value="">All</option>
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {kindLabel(k)}
            </option>
          ))}
        </select>
      </label>
      <label>
        Filter by id prefix
        <input
          type="search"
          value={query.q ?? ''}
          maxLength={MAX_PREFIX_CHARS}
          onChange={(e) => onQuery({ ...query, q: e.target.value === '' ? undefined : e.target.value })}
        />
      </label>
      <label>
        Sort
        <select value={query.sort ?? 'status'} onChange={(e) => onQuery({ ...query, sort: pick(e.target.value, SORTS.map(([v]) => v)) })}>
          {SORTS.map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}
