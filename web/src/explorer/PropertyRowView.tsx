import type { ReactNode } from 'react';
import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { RateCell } from '../components/RateCell';
import { seedPath } from '../router';
import { seedsPath, type SeedFilters } from '../seeds/filters';
import type { PropertyRow } from '../types';
import { formatCount, seedListLabel, seenInInconclusive, statusLabel, statusMark } from '../wording';

/** Height of every row, which is what makes windowing a sum. */
export const ROW_HEIGHT = 88;
/** Vertical padding (2 x 8px) plus the 1px row border, set in explorer.css. */
const ROW_CHROME = 17;
/** Every cell's content sits in a block of exactly this height with overflow hidden, so no row can grow. */
export const CLIP_HEIGHT = ROW_HEIGHT - ROW_CHROME;
const ID_CHARS = 96;
const MSG_CHARS = 140;
const LOC_CHARS = 80;

export const kindLabel = (kind: string): string => kind.replace(/_/g, ' ');

function Clip({ children }: { children: ReactNode }) {
  return (
    <div className="cell-clip" style={{ height: CLIP_HEIGHT }}>
      {children}
    </div>
  );
}

function Location({ row }: { row: PropertyRow }) {
  const loc = row.guest.loc;
  if (!loc) return null;
  return (
    <>
      {' '}
      <GuestText text={loc.file} max={LOC_CHARS} expandable={false} />
      {`:${loc.line}`}
      {loc.func !== null && (
        <>
          {' '}
          <GuestText text={loc.func} max={LOC_CHARS} expandable={false} />
        </>
      )}
    </>
  );
}

/** The seed is a server-built decimal string, linked to its failure page only if it is plain digits. */
function FirstFailingSeed({ runId, seed }: { runId: string | undefined; seed: string | null }) {
  if (seed === null) return <>–</>;
  const to = runId === undefined ? undefined : seedPath(runId, seed);
  return to === undefined ? <>{`seed ${seed}`}</> : <Link to={to}>{`seed ${seed}`}</Link>;
}

/**
 * A link from a property to the seed explorer filtered by it: the seeds that failed a failing property, the seeds that
 * never reached an UNMET one, and otherwise the seeds that reached it. The property is named by its catalog index.
 */
function SeedList({ runId, row }: { runId: string | undefined; row: PropertyRow }) {
  if (runId === undefined) return null;
  const to = seedsPath(runId, listFilter(row));
  if (to === undefined) return null;
  return (
    <span className="clip-line">
      <Link to={to}>{seedListLabel(row.status)}</Link>
    </span>
  );
}

function listFilter(row: PropertyRow): SeedFilters {
  if (row.status === 'violation') return { fails: row.idx };
  return row.status === 'unmet' ? { misses: row.idx } : { hits: row.idx };
}

/** One property as a table row. Guest strings (id, message, location) go through `GuestText` and nowhere else. */
export function PropertyRowView({ row, rowIndex, runId }: { row: PropertyRow; rowIndex: number; runId?: string }) {
  return (
    <tr aria-rowindex={rowIndex} style={{ height: ROW_HEIGHT }}>
      <td className={`status status-${row.status}`}>
        <Clip>
          <span aria-hidden="true">{statusMark(row.status)}</span> {statusLabel(row.status)}
        </Clip>
      </td>
      <td>
        <Clip>{kindLabel(row.kind)}</Clip>
      </td>
      <th scope="row" className="prop-id">
        <Clip>
          <span className="clip-line prop-name">
            <GuestText text={row.guest.id} max={ID_CHARS} expandable={false} />
          </span>
          <span className="clip-line prop-sub">
            {row.guest.msg !== '' && <GuestText text={row.guest.msg} max={MSG_CHARS} expandable={false} />}
            <Location row={row} />
          </span>
        </Clip>
      </th>
      <td>
        <Clip>
          <RateCell cell={row.rate} />
          {row.unmet !== null && (
            <span className="clip-line unmet-note" title={row.unmet}>
              {row.unmet}
            </span>
          )}
          {row.failures_seen_in_inconclusive_trials > 0 && (
            <span className="clip-line inconclusive-note">{seenInInconclusive(row.failures_seen_in_inconclusive_trials)}</span>
          )}
        </Clip>
      </td>
      <td>
        <Clip>
          <span className="clip-line">
            <FirstFailingSeed runId={runId} seed={row.first_failing_seed} />
          </span>
          <SeedList runId={runId} row={row} />
        </Clip>
      </td>
      <td className="num">
        <Clip>{formatCount(row.evals)}</Clip>
      </td>
    </tr>
  );
}
