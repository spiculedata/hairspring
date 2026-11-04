import { GuestText } from '../components/GuestText';
import { outputOmitted, outputTrimmed } from '../wording';
import { OUTPUT_LINE_CHARS, prepareTail } from './output';
import type { StreamName } from './types';

const ERROR_CHARS = 300;

export type TailState =
  | { status: 'loading' }
  | { status: 'absent' }
  | { status: 'error'; error: string }
  | { status: 'ready'; text: string };

interface Props {
  stream: StreamName;
  state: TailState;
  /** Bytes the capture cut from the middle of the stream (its head and tail halves were kept). */
  omittedBytes: number;
  /** Indexes (into the lines shown) of the lines the reader's search matched. */
  hits: ReadonlySet<number>;
  /** Show only the matched lines, each keeping its own number. */
  onlyHits: boolean;
  onDownload: (stream: StreamName) => void;
}

function Lines({ stream, lines, hits, onlyHits }: { stream: StreamName; lines: readonly string[]; hits: ReadonlySet<number>; onlyHits: boolean }) {
  return (
    <pre className="output-tail" tabIndex={0} aria-label={`${stream} tail`}>
      {lines.map((line, i) =>
        onlyHits && !hits.has(i) ? null : (
          <span key={i} className={hits.has(i) ? 'out-line out-hit' : 'out-line'}>
            <span className="ln" aria-hidden="true">
              {i + 1}
            </span>
            <GuestText text={line} max={OUTPUT_LINE_CHARS} expandable={false} />
            {'\n'}
          </span>
        ),
      )}
    </pre>
  );
}

function Body({ stream, state, hits, onlyHits }: Pick<Props, 'stream' | 'state' | 'hits' | 'onlyHits'>) {
  if (state.status === 'loading') return <p className="muted">Loading output…</p>;
  if (state.status === 'absent') return <p className="muted">Nothing was kept for this stream.</p>;
  if (state.status === 'error') {
    return (
      <p className="error" role="alert">
        Output could not be loaded: <GuestText text={state.error} max={ERROR_CHARS} expandable={false} />
      </p>
    );
  }
  const tail = prepareTail(state.text);
  if (tail.lines.length === 0 && tail.charsDropped === 0) return <p className="muted">This stream was empty.</p>;
  return (
    <>
      {(tail.charsDropped > 0 || tail.linesDropped > 0) && <p className="muted">{outputTrimmed(tail.charsDropped, tail.linesDropped)}</p>}
      <p className="muted">Line numbers count from the first line shown here.</p>
      <Lines stream={stream} lines={tail.lines} hits={hits} onlyHits={onlyHits} />
    </>
  );
}

/** One kept output stream: escaped text only, bounded, with what was cut said aloud and a download for the rest. */
export function OutputPanel({ stream, state, omittedBytes, hits, onlyHits, onDownload }: Props) {
  return (
    <div className="output-stream">
      <p className="muted">{stream} (escaped, last 64 KiB at most)</p>
      {omittedBytes > 0 && <p className="muted">{outputOmitted(omittedBytes)}.</p>}
      <Body stream={stream} state={state} hits={hits} onlyHits={onlyHits} />
      <button type="button" onClick={() => onDownload(stream)}>
        {`Download ${stream}`}
      </button>
    </div>
  );
}
