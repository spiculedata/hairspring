import { useState } from 'react';
import { GuestText } from '../components/GuestText';
import { formatCount, LANE_SCALE_NOTE, PER_THREAD_NOTE } from '../wording';
import type { LaneMark, LaneRow } from './model';

/** Lanes drawn at first and per "show more"; a trial can have thousands of threads. */
export const LANES_PER_PAGE = 200;
const NAME_CHARS = 80;
const PERCENT = 100;

/** Lanes with a witness first, then the rest in the order they were reported. */
export function orderLanes(rows: readonly LaneRow[]): LaneRow[] {
  return [...rows.filter((r) => r.marks.length > 0), ...rows.filter((r) => r.marks.length === 0)];
}

/** A decoration only: the same facts are in the cells beside it, so it is hidden from assistive technology. */
function Lane({ marks }: { marks: readonly LaneMark[] }) {
  return (
    <span className="lane" aria-hidden="true">
      {marks.map((m) => (
        <span key={m.ord} className="lane-mark" style={{ left: `${(m.position * PERCENT).toFixed(2)}%` }}>
          ✕
        </span>
      ))}
    </span>
  );
}

function WitnessCell({ marks }: { marks: readonly LaneMark[] }) {
  if (marks.length === 0) return <>–</>;
  return (
    <>
      {marks.map((m) => (
        <span key={m.ord} className="clip-line">
          <span aria-hidden="true">✕ </span>
          witness at evaluation {formatCount(m.ord)}:{' '}
          {m.propertyIds.slice(0, 3).map((id, i) => (
            <span key={id}>
              {i > 0 && ', '}
              <GuestText text={id} max={NAME_CHARS} expandable={false} />
            </span>
          ))}
          {m.propertyIds.length > 3 && `, and ${formatCount(m.propertyIds.length - 3)} more`}
        </span>
      ))}
    </>
  );
}

/**
 * One row per thread, as a table: the table is the text alternative to the bars. Each bar is drawn to its own thread's
 * evaluation count, so lanes are never compared with one another and none is aligned in time.
 */
export function ThreadLanes({ rows }: { rows: readonly LaneRow[] }) {
  const [shown, setShown] = useState(LANES_PER_PAGE);
  const ordered = orderLanes(rows);
  const visible = ordered.slice(0, shown);
  return (
    <section aria-labelledby="threads-title">
      <h2 id="threads-title">Threads</h2>
      <p className="muted">{PER_THREAD_NOTE}</p>
      <p className="muted">{LANE_SCALE_NOTE}</p>
      <div className="table-scroll" role="region" aria-label="Threads, scrollable" tabIndex={0}>
        <table className="lanes" aria-label="Threads" aria-rowcount={visible.length + 1}>
          <thead>
            <tr>
              <th scope="col">Process</th>
              <th scope="col">Thread</th>
              <th scope="col">Evaluations</th>
              <th scope="col">Tape consumed</th>
              <th scope="col">Lane (own count)</th>
              <th scope="col">Witness</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((r, i) => (
              <tr key={i}>
                <td>
                  <GuestText text={r.proc} max={NAME_CHARS} expandable={false} />
                </td>
                <th scope="row">
                  <GuestText text={r.thread} max={NAME_CHARS} expandable={false} /> <span className="muted">({r.keyStable ? 'stable key' : 'unstable key'})</span>
                </th>
                <td className="num">{formatCount(r.evals)}</td>
                <td className="num">{formatCount(r.tape)} B</td>
                <td>
                  <Lane marks={r.marks} />
                </td>
                <td>
                  <WitnessCell marks={r.marks} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {ordered.length > shown && (
        <p>
          <button type="button" onClick={() => setShown((n) => n + LANES_PER_PAGE)}>
            Show more threads ({formatCount(shown)} of {formatCount(ordered.length)} shown)
          </button>
        </p>
      )}
    </section>
  );
}
