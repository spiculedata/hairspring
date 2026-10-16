import { ErrorLine } from '../components/ErrorLine';
import {
  codeLabel,
  codeMark,
  formatCount,
  LOADING_TRIALS,
  STRIP_COUNTS_CAPTION,
  STRIP_NOTE,
  STRIP_TITLE,
  stripSummary,
  stripTruncated,
} from '../wording';
import { StripCanvas } from './StripCanvas';
import { countCodes, MAX_STRIP_TRIALS, STRIP_CODES, type StripState } from './strip';

interface Props {
  strip: StripState;
  loading: boolean;
  error: string | undefined;
  /** Open the table at this trial. */
  onOpen: (trial: number) => void;
}

/** The counts as a table: the strip's text alternative, and its legend (each class with its mark and its count). */
function CountsTable({ codes }: { codes: string }) {
  const counts = countCodes(codes);
  const present = STRIP_CODES.filter((c) => counts[c] > 0);
  if (present.length === 0) return null;
  return (
    <table className="strip-counts">
      <caption>{STRIP_COUNTS_CAPTION}</caption>
      <thead>
        <tr>
          <th scope="col">Class</th>
          <th scope="col" className="num">
            Trials
          </th>
        </tr>
      </thead>
      <tbody>
        {present.map((c) => (
          <tr key={c}>
            <th scope="row" className={`class-${c}`}>
              {`${codeMark(c)} ${codeLabel(c)}`}
            </th>
            <td className="num">{formatCount(counts[c])}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The strip of judged trials with its words: summary, how to read it, the counts table, and any cut or error. */
export function StripSection({ strip, loading, error, onOpen }: Props) {
  return (
    <section aria-labelledby="strip-title">
      <h2 id="strip-title">{STRIP_TITLE}</h2>
      <p>{stripSummary(countCodes(strip.codes))}</p>
      <p className="muted">{STRIP_NOTE}</p>
      <StripCanvas codes={strip.codes} onOpen={onOpen} />
      {loading && <p className="muted">{LOADING_TRIALS}</p>}
      {strip.truncated && <p className="note">{stripTruncated(MAX_STRIP_TRIALS)}</p>}
      {error !== undefined && <ErrorLine message={error} />}
      <CountsTable codes={strip.codes} />
    </section>
  );
}
