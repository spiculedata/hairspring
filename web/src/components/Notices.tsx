import { Link } from './Layout';
import { GuestText } from './GuestText';
import { useConnectionLoss } from './useConnectionLoss';

const ERROR_CHARS = 300;

/** What a page shows while its data loads: it says what is loading. */
export function Loading({ what }: { what: string }) {
  return <p className="muted">{`Loading ${what}…`}</p>;
}

/** What a page shows when the thing it is about is not there: what is missing, why that may be, and where to go next. */
export function NotFound({ what }: { what: 'run' | 'trial' | 'failure' | 'page' }) {
  return (
    <>
      <h1>{`No such ${what}`}</h1>
      <p className="error" role="alert">
        {what === 'page' ? (
          <>There is no page at this address. Check the address, or go to <Link to="/runs">Runs</Link>.</>
        ) : (
          <>{`This ${what} does not exist here. It may have been removed.`} Go back to <Link to="/runs">Runs</Link>.</>
        )}
      </p>
    </>
  );
}

/**
 * A load that failed: what was being loaded, the reason, and what to do about it, all in one announced alert. A page
 * that polls tries again by itself, so it does not tell the reader to reload.
 */
export function LoadFailed({ what, message, network = false, retries = true }: { what: string; message: string; network?: boolean; retries?: boolean }) {
  const loss = useConnectionLoss();
  // While the connection banner is up the cause is known and announced there; the browser's own text ("Failed to fetch")
  // would only repeat it in worse words, once per section. An error the server sent is its own answer and is kept: the
  // connection is not what failed for that section, whatever another request is doing.
  if (loss !== undefined && network) return <p className="muted">{`Could not load ${what} until the connection returns.`}</p>;
  return (
    <div className="error" role="alert">
      {`Could not load ${what}: `}
      <GuestText text={message} max={ERROR_CHARS} expandable={false} /> {retries ? 'This page tries again by itself.' : 'Reload the page to try again.'}
    </div>
  );
}
