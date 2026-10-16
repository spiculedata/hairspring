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
  onDownload: (stream: StreamName) => void;
}

function Body({ stream, state }: { stream: StreamName; state: TailState }) {
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
      <pre className="output-tail" tabIndex={0} aria-label={`${stream} tail`}>
        {tail.lines.map((line, i) => (
          <span key={i}>
            <GuestText text={line} max={OUTPUT_LINE_CHARS} expandable={false} />
            {'\n'}
          </span>
        ))}
      </pre>
    </>
  );
}

/** One kept output stream: escaped text only, bounded, with what was cut said aloud and a download for the rest. */
export function OutputTail({ stream, state, omittedBytes, onDownload }: Props) {
  return (
    <div className="output-stream">
      <h3>
        {stream} <span className="muted">(escaped, last 64 KiB at most)</span>
      </h3>
      {omittedBytes > 0 && <p className="muted">{outputOmitted(omittedBytes)}.</p>}
      <Body stream={stream} state={state} />
      <button type="button" onClick={() => onDownload(stream)}>
        {`Download ${stream}`}
      </button>
    </div>
  );
}
