import { useState } from 'react';
import type { PropertyRow } from '../types';
import { computeWindow } from '../windowing';
import { GUEST_TABLE_NOTE, NO_PROPERTIES } from '../wording';
import { PropertyRowView, ROW_HEIGHT } from './PropertyRowView';

const VIEWPORT_HEIGHT = 560;
const OVERSCAN = 6;
const COLUMNS = 6;
/** Rows above the first data row: the column headers. */
const HEADER_ROWS = 1;

function Spacer({ height }: { height: number }) {
  if (height <= 0) return null;
  return (
    <tr aria-hidden="true" style={{ height }}>
      <td colSpan={COLUMNS} />
    </tr>
  );
}

/**
 * The property table. A run can have thousands of properties, so only the rows in view are rendered: a fixed row
 * height lets blank spacer rows stand in for the rest, and `aria-rowcount` / `aria-rowindex` keep the full size
 * available to assistive technology. It stays a real table so keyboard and screen reader navigation work.
 */
export function PropertyTable({ properties, total }: { properties: readonly PropertyRow[]; total?: number }) {
  const [scrollTop, setScrollTop] = useState(0);
  if (properties.length === 0) return <p className="muted">{NO_PROPERTIES}</p>;
  const win = computeWindow({
    total: properties.length,
    rowHeight: ROW_HEIGHT,
    viewportHeight: VIEWPORT_HEIGHT,
    scrollTop,
    overscan: OVERSCAN,
  });
  return (
    <div
      className="table-scroll"
      role="region"
      aria-label="Properties"
      tabIndex={0}
      style={{ maxHeight: VIEWPORT_HEIGHT }}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      <table className="props" aria-rowcount={(total ?? properties.length) + HEADER_ROWS}>
        <caption>{GUEST_TABLE_NOTE}</caption>
        <thead>
          <tr aria-rowindex={1}>
            <th scope="col">Status</th>
            <th scope="col">Kind</th>
            <th scope="col">Property</th>
            <th scope="col">Rate (rated trials)</th>
            <th scope="col">First failing seed</th>
            <th scope="col" className="num">
              Evals
            </th>
          </tr>
        </thead>
        <tbody>
          <Spacer height={win.padTop} />
          {properties.slice(win.start, win.end).map((row, i) => (
            <PropertyRowView key={row.idx} row={row} rowIndex={win.start + i + HEADER_ROWS + 1} />
          ))}
          <Spacer height={win.padBottom} />
        </tbody>
      </table>
    </div>
  );
}
