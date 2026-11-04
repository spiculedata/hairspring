import { useEffect, useId, useState, type FormEvent, type ReactNode } from 'react';
import { FINDING_STATUSES } from '../types';
import {
  FAILURE_FILTERS_LABEL,
  FAILURE_PROPERTY_LABEL,
  FAILURE_RUN_LABEL,
  FAILURE_STATUS_LABEL,
  FILTER_ALL,
  FILTERS_CLEAR,
  PROPERTY_FILTER_HINT,
  RUN_FILTERS_LABEL,
  RUN_MODE_LABEL,
  RUN_STATE_LABEL,
  RUN_TARGET_LABEL,
  STATUS_FILTER_NOTE,
  findingOption,
  problemText,
  runModeOption,
  runStateOption,
} from './listWording';
import {
  MAX_PROPERTY_PREFIX_BYTES,
  MAX_TARGET_CHARS,
  RUN_MODE_FILTERS,
  RUN_STATE_FILTERS,
  checkPropertyPrefix,
  checkRunId,
  checkTarget,
  hasFailureFilters,
  hasRunFilters,
  type FailureFilters,
  type FilterProblem,
  type RunFilters,
} from './filters';

type Checked = { value: string | undefined } | { problem: FilterProblem };

/** One value of a `<select>` narrowed to one of the options, or `undefined` for "all". */
const pick = <T extends string>(value: string, options: readonly T[]): T | undefined => options.find((o) => o === value);

interface TextFieldProps {
  readonly label: string;
  /** The value now in force, which the box shows until the reader types. */
  readonly value: string | undefined;
  readonly check: (text: string) => Checked;
  /** Called with the checked value (undefined when the box was emptied). Never called with a value that is not valid. */
  readonly onValue: (value: string | undefined) => void;
  /** Characters the box accepts, a guide only: the check decides. */
  readonly maxLength: number;
  readonly hint?: string;
}

/** A text box that applies on Enter and when it loses focus, and says what is wrong instead of sending a value the server would refuse. */
function TextField({ label, value, check, onValue, maxLength, hint }: TextFieldProps) {
  const [text, setText] = useState(value ?? '');
  const [problem, setProblem] = useState<FilterProblem>();
  const inputId = useId();
  const problemId = useId();
  const hintId = useId();
  // A change made elsewhere (Clear filters, a link) replaces what was typed.
  useEffect(() => {
    setText(value ?? '');
    setProblem(undefined);
  }, [value]);
  const apply = () => {
    const checked = check(text);
    if ('problem' in checked) return setProblem(checked.problem);
    setProblem(undefined);
    if (checked.value !== value) onValue(checked.value);
  };
  const describedBy = [problem === undefined ? undefined : problemId, hint === undefined ? undefined : hintId].filter(Boolean).join(' ');
  return (
    <div className="filter-field">
      <label htmlFor={inputId}>{label}</label>
      <input
        id={inputId}
        type="search"
        value={text}
        maxLength={maxLength}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={problem !== undefined}
        aria-describedby={describedBy === '' ? undefined : describedBy}
        onChange={(e) => setText(e.target.value)}
        onBlur={apply}
        onKeyDown={(e) => {
          if (e.key !== 'Enter') return;
          e.preventDefault();
          apply();
        }}
      />
      {hint !== undefined && (
        <span id={hintId} className="muted filter-hint">
          {hint}
        </span>
      )}
      {problem !== undefined && (
        <span id={problemId} className="error filter-problem" role="alert">
          {problemText(problem)}
        </span>
      )}
    </div>
  );
}

function Bar({ label, children }: { label: string; children: ReactNode }) {
  return (
    <form className="row filters list-filters" role="search" aria-label={label} onSubmit={(e: FormEvent) => e.preventDefault()}>
      {children}
    </form>
  );
}

interface RunBarProps {
  readonly filters: RunFilters;
  readonly onChange: (next: RunFilters) => void;
}

/** The filters of the Runs list. State and run type apply when chosen; the target applies on Enter or when the box loses focus. */
export function RunFilterBar({ filters, onChange }: RunBarProps) {
  return (
    <Bar label={RUN_FILTERS_LABEL}>
      <label className="filter-field">
        {RUN_STATE_LABEL}
        <select value={filters.state ?? ''} onChange={(e) => onChange({ ...filters, state: pick(e.target.value, RUN_STATE_FILTERS) })}>
          <option value="">{FILTER_ALL}</option>
          {RUN_STATE_FILTERS.map((s) => (
            <option key={s} value={s}>
              {runStateOption(s)}
            </option>
          ))}
        </select>
      </label>
      <label className="filter-field">
        {RUN_MODE_LABEL}
        <select value={filters.mode ?? ''} onChange={(e) => onChange({ ...filters, mode: pick(e.target.value, RUN_MODE_FILTERS) })}>
          <option value="">{FILTER_ALL}</option>
          {RUN_MODE_FILTERS.map((m) => (
            <option key={m} value={m}>
              {runModeOption(m)}
            </option>
          ))}
        </select>
      </label>
      <TextField
        label={RUN_TARGET_LABEL}
        value={filters.target}
        check={checkTarget}
        maxLength={MAX_TARGET_CHARS}
        onValue={(target) => onChange({ ...filters, target })}
      />
      <Actions show={hasRunFilters(filters)} onClear={() => onChange({})} />
    </Bar>
  );
}

interface FailureBarProps {
  readonly filters: FailureFilters;
  readonly onChange: (next: FailureFilters) => void;
}

/** The filters of the Failures list. The state applies when chosen; the run id and the property prefix apply on Enter or when the box loses focus. */
export function FailureFilterBar({ filters, onChange }: FailureBarProps) {
  return (
    <Bar label={FAILURE_FILTERS_LABEL}>
      <TextField
        label={FAILURE_RUN_LABEL}
        value={filters.run}
        check={checkRunId}
        maxLength={'run_'.length + 16}
        onValue={(run) => onChange({ ...filters, run })}
      />
      <TextField
        label={FAILURE_PROPERTY_LABEL}
        value={filters.property}
        check={checkPropertyPrefix}
        maxLength={MAX_PROPERTY_PREFIX_BYTES}
        hint={PROPERTY_FILTER_HINT}
        onValue={(property) => onChange({ ...filters, property })}
      />
      <label className="filter-field">
        {FAILURE_STATUS_LABEL}
        <select
          value={filters.status ?? ''}
          onChange={(e) => onChange({ ...filters, status: pick(e.target.value, FINDING_STATUSES) })}
        >
          <option value="">{FILTER_ALL}</option>
          {FINDING_STATUSES.map((s) => (
            <option key={s} value={s}>
              {findingOption(s)}
            </option>
          ))}
        </select>
      </label>
      <Actions show={hasFailureFilters(filters)} onClear={() => onChange({})} />
      {filters.status !== undefined && <p className="muted filter-note">{STATUS_FILTER_NOTE}</p>}
    </Bar>
  );
}

function Actions({ show, onClear }: { show: boolean; onClear: () => void }) {
  return (
    <div className="filter-actions">
      {show && (
        <button type="button" className="secondary" onClick={onClear}>
          {FILTERS_CLEAR}
        </button>
      )}
    </div>
  );
}
