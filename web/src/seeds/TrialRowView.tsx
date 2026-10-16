import type { ReactNode } from 'react';
import { GuestText } from '../components/GuestText';
import { Link } from '../components/Layout';
import { trialPath } from '../router';
import type { TrialRow } from '../trial/types';
import { formatCount, keptText, moreFailing, NO_VALUE, verdictCellText, verdictTitle } from '../wording';
import { isFingerprint, type SeedFilters } from './filters';
import { CLIP_HEIGHT, ROW_HEIGHT } from './rowMetrics';

/** Failing ids named inline on a row; the rest are counted. */
const IDS_SHOWN = 2;
const ID_CHARS = 40;
/** Leading hex digits of a fingerprint shown in the table; the filter and the tooltip use all of it. */
const FP_SHOWN = 8;
/** A seed is a decimal string the server built from a 64-bit number; anything else is shown as escaped text. */
const DIGITS = /^\d{1,20}$/;
const SEED_CHARS = 24;

function Clip({ children }: { children: ReactNode }) {
  return (
    <div className="cell-clip" style={{ height: CLIP_HEIGHT }}>
      <span className="clip-line">{children}</span>
    </div>
  );
}

interface Props {
  row: TrialRow;
  rowIndex: number;
  runId: string;
  highlight: boolean;
  onFilter: (f: Partial<SeedFilters>) => void;
}

function FailingIds({ row, onFilter }: Pick<Props, 'row' | 'onFilter'>) {
  if (row.failing_count === 0) return <>{NO_VALUE}</>;
  const shown = row.guest.failing_ids.slice(0, IDS_SHOWN);
  const extra = row.failing_count - shown.length;
  return (
    <>
      {shown.map((id, i) => {
        const idx = row.failing_idx[i];
        return (
          <span key={i}>
            {i > 0 && ', '}
            {idx === undefined ? (
              <GuestText text={id} max={ID_CHARS} expandable={false} />
            ) : (
              <button type="button" className="linklike id-filter" onClick={() => onFilter({ fails: idx })}>
                <GuestText text={id} max={ID_CHARS} expandable={false} />
              </button>
            )}
          </span>
        );
      })}
      {extra > 0 && ` ${moreFailing(extra)}`}
    </>
  );
}

function Fingerprint({ fp, onFilter }: { fp: string | null; onFilter: Props['onFilter'] }) {
  if (fp === null) return <>{NO_VALUE}</>;
  if (!isFingerprint(fp)) return <GuestText text={fp} max={FP_SHOWN} expandable={false} />;
  return (
    <button type="button" className="linklike fp-filter" title={fp} onClick={() => onFilter({ fp: fp.toLowerCase() })}>
      {fp.slice(0, FP_SHOWN)}…
    </button>
  );
}

/** One judged trial as a table row. Guest strings (the failing ids) go through `GuestText` and nowhere else. */
export function TrialRowView({ row, rowIndex, runId, highlight, onFilter }: Props) {
  const to = trialPath(runId, row.seq);
  return (
    <tr aria-rowindex={rowIndex} style={{ height: ROW_HEIGHT }} className={highlight ? 'start-row' : undefined}>
      <td className="num">
        <Clip>{to === undefined ? row.seq : <Link to={to}>{row.seq}</Link>}</Clip>
      </td>
      <td className="mono">
        <Clip>{DIGITS.test(row.seed) ? row.seed : <GuestText text={row.seed} max={SEED_CHARS} expandable={false} />}</Clip>
      </td>
      <th scope="row" className={`verdict class-${row.class}`} title={verdictTitle(row)}>
        <Clip>{verdictCellText(row)}</Clip>
      </th>
      <td>
        <Clip>
          <FailingIds row={row} onFilter={onFilter} />
        </Clip>
      </td>
      <td className="mono">
        <Clip>
          <Fingerprint fp={row.schedule_fp} onFilter={onFilter} />
        </Clip>
      </td>
      <td className="num">
        <Clip>{formatCount(row.work_ops)}</Clip>
      </td>
      <td className="num">
        <Clip>{formatCount(row.elapsed_ms)}</Clip>
      </td>
      <td>
        <Clip>{keptText(row.kept)}</Clip>
      </td>
    </tr>
  );
}
