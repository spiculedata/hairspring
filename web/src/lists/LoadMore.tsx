import { GuestText } from '../components/GuestText';
import { LOADING_MORE, loadMoreFailed, shownAnnouncement } from './listWording';

interface Props {
  /** The label of the button, which says what is shown so far. */
  readonly label: string;
  readonly what: 'runs' | 'failures';
  readonly shown: number;
  /** An older page exists. Without one, nothing is drawn but the live note. */
  readonly hasMore: boolean;
  readonly loading: boolean;
  readonly onLoadMore: () => void;
  /** The reason the last attempt failed; the rows already shown stay. */
  readonly error: string | undefined;
  /** Pages added so far; the live note says nothing until the reader has asked for one. */
  readonly moreLoads: number;
}

const ERROR_CHARS = 300;

/**
 * The control that adds older rows to a list. The button is a real button and is disabled while a request is out. A
 * failure is announced where it happens and the button stays, so asking again tries again. A polite live note says how
 * many rows are shown after the reader asked for more; it says nothing while the list merely refreshes.
 */
export function LoadMore({ label, what, shown, hasMore, loading, onLoadMore, error, moreLoads }: Props) {
  return (
    <div className="load-more">
      {error !== undefined && (
        <div className="error" role="alert">
          <GuestText text={loadMoreFailed(what, error)} max={ERROR_CHARS} expandable={false} />
        </div>
      )}
      {hasMore && (
        <p>
          <button type="button" onClick={onLoadMore} disabled={loading} aria-busy={loading}>
            {loading ? LOADING_MORE : label}
          </button>
        </p>
      )}
      <p className="sr-only" role="status">
        {moreLoads > 0 ? shownAnnouncement(shown, what) : ''}
      </p>
    </div>
  );
}
