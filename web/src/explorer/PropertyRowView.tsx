import { GuestText } from '../components/GuestText';
import { RateCell } from '../components/RateCell';
import type { PropertyRow } from '../types';
import { formatCount, seenInInconclusive, statusLabel, statusMark } from '../wording';

/** Fixed height of every row, which is what makes windowing a sum. Cells clip to it. */
export const ROW_HEIGHT = 56;
const ID_CHARS = 96;
const MSG_CHARS = 140;
const LOC_CHARS = 80;

export const kindLabel = (kind: string): string => kind.replace(/_/g, ' ');

function Location({ row }: { row: PropertyRow }) {
  const loc = row.guest.loc;
  if (!loc) return null;
  return (
    <span className="prop-loc">
      <GuestText text={loc.file} max={LOC_CHARS} expandable={false} />
      {`:${loc.line}`}
      {loc.func !== null && (
        <>
          {' '}
          <GuestText text={loc.func} max={LOC_CHARS} expandable={false} />
        </>
      )}
    </span>
  );
}

/** One property as a table row. Guest strings (id, message, location) go through `GuestText` and nowhere else. */
export function PropertyRowView({ row, rowIndex }: { row: PropertyRow; rowIndex: number }) {
  return (
    <tr aria-rowindex={rowIndex} style={{ height: ROW_HEIGHT }}>
      <td className={`status status-${row.status}`}>
        <span aria-hidden="true">{statusMark(row.status)}</span> {statusLabel(row.status)}
      </td>
      <td>{kindLabel(row.kind)}</td>
      <th scope="row" className="prop-id">
        <GuestText text={row.guest.id} max={ID_CHARS} expandable={false} />
        {row.guest.msg !== '' && (
          <span className="prop-msg">
            <GuestText text={row.guest.msg} max={MSG_CHARS} expandable={false} />
          </span>
        )}
        <Location row={row} />
      </th>
      <td>
        <RateCell cell={row.rate} />
        {row.unmet !== null && <span className="unmet-note">{row.unmet}</span>}
        {row.failures_seen_in_inconclusive_trials > 0 && (
          <span className="inconclusive-note">{seenInInconclusive(row.failures_seen_in_inconclusive_trials)}</span>
        )}
      </td>
      <td>{row.first_failing_seed === null ? '–' : `seed ${row.first_failing_seed}`}</td>
      <td className="num">{formatCount(row.evals)}</td>
    </tr>
  );
}
