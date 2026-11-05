import { useLayoutEffect, useRef } from 'react';
import { GuestText } from '../components/GuestText';
import { allShown, LIST_DEPTH_REACHED, LOADING_MORE, loadMoreFailed, shownAnnouncement } from './listWording';

interface Props {
  /** The label of the button, which says what is shown so far. */
  readonly label: string;
  readonly what: 'runs' | 'failures';
  readonly shown: number;
  /** An older page exists. Without one, nothing is drawn but the live note. */
  readonly hasMore: boolean;
  readonly loading: boolean;
  /** The server stopped the list: it cannot be read any further back. A note, not a failure. */
  readonly endReached: boolean;
  readonly onLoadMore: () => void;
  /** The reason the last attempt failed; the rows already shown stay. */
  readonly error: string | undefined;
  /** Pages added so far; the live note says nothing until the reader has asked for one. */
  readonly moreLoads: number;
}

const ERROR_CHARS = 300;

/**
 * The control that adds older rows to a list. The button is a real button. While a request is out it is `aria-disabled`
 * and not `disabled`, so a keyboard user keeps their place. When the last page arrives the button goes, and focus moves
 * to the note that says all rows are shown, if the button had it. A failure is announced where it happens and the button
 * stays, so asking again tries again. A polite live note says how many rows are shown after the reader asked for more;
 * it says nothing while the list merely refreshes.
 */
export function LoadMore({ label, what, shown, hasMore, loading, endReached, onLoadMore, error, moreLoads }: Props) {
  const hadFocus = useRef(false);
  const endNote = useRef<HTMLParagraphElement>(null);
  // The button leaves when there is nothing older: keep focus in the page by moving it to the note that takes its place.
  useLayoutEffect(() => {
    if (hasMore || !hadFocus.current) return;
    hadFocus.current = false;
    endNote.current?.focus();
  }, [hasMore]);
  const showEnd = !hasMore && moreLoads > 0 && error === undefined;
  return (
    <div className="load-more">
      {error !== undefined && (
        <div className="error" role="alert">
          <GuestText text={loadMoreFailed(what, error)} max={ERROR_CHARS} expandable={false} />
        </div>
      )}
      {hasMore && (
        <p>
          <button
            type="button"
            onClick={() => {
              if (!loading) onLoadMore();
            }}
            onFocus={() => {
              hadFocus.current = true;
            }}
            onBlur={() => {
              hadFocus.current = false;
            }}
            aria-disabled={loading}
            aria-busy={loading}
          >
            {loading ? LOADING_MORE : label}
          </button>
        </p>
      )}
      {showEnd && (
        <p className="muted" tabIndex={-1} ref={endNote}>
          {endReached ? LIST_DEPTH_REACHED : allShown(shown, what)}
        </p>
      )}
      <p className="sr-only" role="status">
        {moreLoads > 0 ? shownAnnouncement(shown, what) : ''}
      </p>
    </div>
  );
}
