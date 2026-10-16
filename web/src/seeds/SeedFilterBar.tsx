import { useState } from 'react';
import { GuestText } from '../components/GuestText';
import type { PropertyRow } from '../types';
import {
  BACK_TO_START,
  classLabel,
  FILTER_ALL,
  FILTERS_CLEAR,
  FP_FILTER_HINT,
  FP_FILTER_INVALID,
  FP_FILTER_LABEL,
  formatCount,
  PROPERTY_PICK_NONE,
  PROPERTY_SEARCH_LABEL,
  propertyModeLabel,
  START_AT_NOTE,
} from '../wording';
import { hasFilters, isFingerprint, propertyOf, VERDICT_FILTERS, withProperty, type PropertyMode, type SeedFilters } from './filters';

const MODES: readonly PropertyMode[] = ['fails', 'hits', 'misses'];
/** The longest id prefix the server accepts, and the longest fingerprint. */
const MAX_PREFIX_CHARS = 128;
const MAX_FP_CHARS = 64;
const ID_CHARS = 80;

export interface PropertySearch {
  text: string;
  matches: readonly PropertyRow[];
  state: 'idle' | 'loading' | 'error';
}

interface Props {
  filters: SeedFilters;
  onChange: (next: SeedFilters) => void;
  /** The id of the property the filter names, once it is known. Guest text. */
  propertyLabel: string | undefined;
  search: PropertySearch;
  onSearch: (text: string) => void;
  /** The text in the fingerprint box, which may not be a valid fingerprint yet. */
  fpText: string;
  onFpText: (text: string) => void;
}

const modeOf = (s: string): PropertyMode | undefined => MODES.find((m) => m === s);

function VerdictSelect({ filters, onChange }: Pick<Props, 'filters' | 'onChange'>) {
  return (
    <label>
      Verdict
      <select
        value={filters.verdict ?? ''}
        onChange={(e) => onChange({ ...filters, verdict: VERDICT_FILTERS.find((v) => v === e.target.value) })}
      >
        <option value="">{FILTER_ALL}</option>
        {VERDICT_FILTERS.map((v) => (
          <option key={v} value={v}>
            {v === 'infra' ? classLabel('infra') : v}
          </option>
        ))}
      </select>
    </label>
  );
}

function PropertyFilter({ filters, onChange, propertyLabel, search, onSearch }: Props) {
  const chosen = propertyOf(filters);
  const [pending, setPending] = useState<PropertyMode>('fails');
  const mode = chosen?.mode ?? pending;
  return (
    <>
      <label>
        Trials where a property
        <select
          value={mode}
          onChange={(e) => {
            const next = modeOf(e.target.value) ?? 'fails';
            setPending(next);
            if (chosen) onChange(withProperty(filters, next, chosen.idx));
          }}
        >
          {MODES.map((m) => (
            <option key={m} value={m}>
              {propertyModeLabel(m)}
            </option>
          ))}
        </select>
      </label>
      {chosen ? (
        <span className="chip chip-info prop-chip">
          {propertyModeLabel(chosen.mode)}{' '}
          {propertyLabel === undefined ? `property ${chosen.idx}` : <GuestText text={propertyLabel} max={ID_CHARS} expandable={false} />}{' '}
          <button type="button" className="linklike" onClick={() => onChange(withProperty(filters, mode, undefined))}>
            remove
          </button>
        </span>
      ) : (
        <label>
          {PROPERTY_SEARCH_LABEL}
          <input type="search" value={search.text} maxLength={MAX_PREFIX_CHARS} onChange={(e) => onSearch(e.target.value)} />
        </label>
      )}
      {!chosen && search.text !== '' && (
        <ul className="prop-matches">
          {search.matches.map((p) => (
            <li key={p.idx}>
              <button type="button" className="linklike" onClick={() => onChange(withProperty(filters, mode, p.idx))}>
                <GuestText text={p.guest.id} max={ID_CHARS} expandable={false} />
              </button>
            </li>
          ))}
          {search.state === 'idle' && search.matches.length === 0 && <li className="muted">{PROPERTY_PICK_NONE}</li>}
        </ul>
      )}
    </>
  );
}

/** The seed explorer's filters. A property is chosen by its id, and the filter then holds its catalog index. */
export function SeedFilterBar(props: Props) {
  const { filters, onChange, fpText, onFpText } = props;
  const fpInvalid = fpText !== '' && !isFingerprint(fpText);
  return (
    <div className="seed-filters">
      <div className="row filters">
        <VerdictSelect filters={filters} onChange={onChange} />
        <PropertyFilter {...props} />
        <label>
          {FP_FILTER_LABEL}
          <input
            type="search"
            value={fpText}
            maxLength={MAX_FP_CHARS}
            placeholder={FP_FILTER_HINT}
            aria-invalid={fpInvalid}
            onChange={(e) => {
              const text = e.target.value.trim();
              onFpText(text);
              if (text === '') onChange({ ...filters, fp: undefined });
              else if (isFingerprint(text)) onChange({ ...filters, fp: text.toLowerCase() });
            }}
          />
        </label>
        {hasFilters(filters) && (
          <button
            type="button"
            onClick={() => {
              onFpText('');
              onChange({});
            }}
          >
            {FILTERS_CLEAR}
          </button>
        )}
      </div>
      {fpInvalid && <p className="error">{FP_FILTER_INVALID}</p>}
      {filters.from !== undefined && (
        <p className="note">
          {`The list starts at trial ${formatCount(filters.from)}. ${START_AT_NOTE}`}{' '}
          <button type="button" className="linklike" onClick={() => onChange({ ...filters, from: undefined })}>
            {BACK_TO_START}
          </button>
        </p>
      )}
    </div>
  );
}
