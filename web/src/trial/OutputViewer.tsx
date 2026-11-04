import { useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { findLines, prepareTail } from './output';
import { OutputPanel, type TailState } from './OutputTail';
import type { StreamName } from './types';

interface Props {
  /** The streams to offer, in tab order. */
  streams: readonly StreamName[];
  tails: Partial<Record<StreamName, TailState>>;
  omittedBytes: Record<StreamName, number>;
  onDownload: (stream: StreamName) => void;
}

const NO_HITS: ReadonlySet<number> = new Set();

/** The tab a key moves to, or undefined for a key that does not move between tabs. */
function targetIndex(key: string, from: number, count: number): number | undefined {
  if (key === 'ArrowRight') return (from + 1) % count;
  if (key === 'ArrowLeft') return (from - 1 + count) % count;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return undefined;
}

function matchText(count: number): string {
  if (count === 0) return 'No matching lines';
  return `${count} matching ${count === 1 ? 'line' : 'lines'}`;
}

function hitsFor(state: TailState | undefined, query: string): ReadonlySet<number> {
  if (state?.status !== 'ready' || query === '') return NO_HITS;
  return new Set(findLines(prepareTail(state.text).lines, query));
}

/**
 * The output of a trial: one stream at a time with a switch between them, numbered lines, and a find box that marks the
 * lines it matches. Every panel stays in the page and the others are hidden, so nothing about a stream is lost by
 * looking at the other. The find text is only ever compared with the lines, never shown as markup.
 */
export function OutputViewer({ streams, tails, omittedBytes, onDownload }: Props) {
  const [active, setActive] = useState<StreamName>(streams[0] ?? 'stdout');
  const [query, setQuery] = useState('');
  const [onlyHits, setOnlyHits] = useState(false);
  const base = useId();
  const tabRefs = useRef(new Map<StreamName, HTMLButtonElement>());
  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = targetIndex(e.key, index, streams.length);
    if (next === undefined) return;
    e.preventDefault();
    const stream = streams[next];
    setActive(stream);
    tabRefs.current.get(stream)?.focus();
  };
  const hitSets = useMemo(() => new Map(streams.map((s) => [s, hitsFor(tails[s], query)])), [streams, tails, query]);
  const activeHits = hitSets.get(active) ?? NO_HITS;
  return (
    <div className="output-viewer">
      <div className="output-tools">
        <div role="tablist" aria-label="Output stream" className="output-tabs">
          {streams.map((s, index) => (
            <button key={s} ref={(el) => { if (el) tabRefs.current.set(s, el); else tabRefs.current.delete(s); }} tabIndex={s === active ? 0 : -1} onKeyDown={(e) => onTabKey(e, index)} type="button" role="tab" id={`${base}-tab-${s}`} aria-selected={s === active} aria-controls={`${base}-panel-${s}`} className={s === active ? 'tab tab-on' : 'tab'} onClick={() => setActive(s)}>
              {s}
            </button>
          ))}
        </div>
        <input type="search" aria-label="Find in output" placeholder="Find in output" value={query} onChange={(e) => setQuery(e.target.value)} />
        <label className="inline-check">
          <input type="checkbox" checked={onlyHits} onChange={(e) => setOnlyHits(e.target.checked)} />
          Only matching lines
        </label>
        {query !== '' && (
          <span className="muted" role="status">
            {matchText(activeHits.size)}
          </span>
        )}
      </div>
      {streams.map((s) => (
        <div key={s} role="tabpanel" id={`${base}-panel-${s}`} aria-labelledby={`${base}-tab-${s}`} hidden={s !== active}>
          <OutputPanel stream={s} state={tails[s] ?? { status: 'absent' }} omittedBytes={omittedBytes[s]} hits={hitSets.get(s) ?? NO_HITS} onlyHits={onlyHits} onDownload={onDownload} />
        </div>
      ))}
    </div>
  );
}
