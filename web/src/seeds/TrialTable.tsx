import { useState } from 'react';
import { ScrollRegion } from '../components/ScrollRegion';
import type { TrialRow } from '../trial/types';
import { computeWindow } from '../windowing';
import { FAILING_COLUMN_LABEL, FAILING_COLUMN_NOTE, GUEST_IDS_NOTE, SCROLL_HINT_SEEDS, TRIAL_TABLE_LABEL } from '../wording';
import { COLUMN_KEYS, COLUMN_WIDTHS_PX, columnBudgetPx } from './columns';
import type { SeedFilters } from './filters';
import { HEADER_ROWS, NEAR_END_ROWS, OVERSCAN, ROW_HEIGHT, VIEWPORT_HEIGHT } from './rowMetrics';
import { TrialRowView } from './TrialRowView';

const COLUMNS = COLUMN_KEYS.length;
/** ARIA's value for a row count that is not known. */
const UNKNOWN_ROWS = -1;

function Spacer({ height }: { height: number }) {
  if (height <= 0) return null;
  return (
    <tr aria-hidden="true" style={{ height }}>
      <td colSpan={COLUMNS} />
    </tr>
  );
}

/** Whether a scroll position is within [`NEAR_END_ROWS`] of the end of `rows` loaded rows. */
export function isNearEnd(scrollTop: number, viewportHeight: number, rows: number): boolean {
  return scrollTop + viewportHeight >= (rows - NEAR_END_ROWS) * ROW_HEIGHT;
}

interface Props {
  runId: string;
  rows: readonly TrialRow[];
  /** How many rows the whole list has, when that is known (an unfiltered list); otherwise the count is "unknown". */
  total: number | undefined;
  /** The trial a jump from the strip landed on. */
  highlightSeq: number | undefined;
  onFilter: (f: Partial<SeedFilters>) => void;
  /** Called when the reader scrolls close to the last loaded row. */
  onNearEnd: () => void;
}

/**
 * The judged trials as a windowed table. Only the rows in view are rendered: a fixed row height (rowMetrics.ts) lets blank
 * spacer rows stand in for the rest, so the scroll height is exactly the number of loaded rows times the row height.
 * `aria-rowcount` and `aria-rowindex` keep the size available to assistive technology, and it stays a real table so
 * keyboard and screen reader navigation work.
 */
export function TrialTable({ runId, rows, total, highlightSeq, onFilter, onNearEnd }: Props) {
  const [scrollTop, setScrollTop] = useState(0);
  if (rows.length === 0) return null;
  const win = computeWindow({ total: rows.length, rowHeight: ROW_HEIGHT, viewportHeight: VIEWPORT_HEIGHT, scrollTop, overscan: OVERSCAN });
  return (
    <ScrollRegion
      label={TRIAL_TABLE_LABEL}
      hint={SCROLL_HINT_SEEDS}
      maxHeight={VIEWPORT_HEIGHT}
      onScroll={(e) => {
        const el = e.currentTarget;
        setScrollTop(el.scrollTop);
        if (isNearEnd(el.scrollTop, el.clientHeight, rows.length)) onNearEnd();
      }}
    >
      <table className="seeds" aria-rowcount={total === undefined ? UNKNOWN_ROWS : total + HEADER_ROWS} style={{ minWidth: columnBudgetPx() }}>
        <caption>
          {GUEST_IDS_NOTE} {FAILING_COLUMN_NOTE}
        </caption>
        <colgroup>
          {COLUMN_KEYS.map((key) => (
            <col key={key} style={{ width: COLUMN_WIDTHS_PX[key] }} />
          ))}
        </colgroup>
        <thead>
          <tr aria-rowindex={1}>
            <th scope="col" className="num">
              Trial
            </th>
            <th scope="col">Seed</th>
            <th scope="col">Verdict</th>
            <th scope="col">{FAILING_COLUMN_LABEL}</th>
            <th scope="col">Schedule</th>
            <th scope="col" className="num">
              Ops
            </th>
            <th scope="col" className="num">
              ms
            </th>
            <th scope="col">Detail kept</th>
          </tr>
        </thead>
        <tbody>
          <Spacer height={win.padTop} />
          {rows.slice(win.start, win.end).map((row, i) => (
            <TrialRowView
              key={row.seq}
              row={row}
              rowIndex={win.start + i + HEADER_ROWS + 1}
              runId={runId}
              highlight={row.seq === highlightSeq}
              onFilter={onFilter}
            />
          ))}
          <Spacer height={win.padBottom} />
        </tbody>
      </table>
    </ScrollRegion>
  );
}
