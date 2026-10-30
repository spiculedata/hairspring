import { Link } from './Layout';
import { ErrorLine } from './ErrorLine';

/** What a page shows while its data loads: it says what is loading. */
export function Loading({ what }: { what: string }) {
  return <p className="muted">{`Loading ${what}…`}</p>;
}

/** What a page shows when the thing it is about is not there: what is missing, why that may be, and where to go next. */
export function NotFound({ what }: { what: 'run' | 'trial' | 'failure' | 'page' }) {
  return (
    <p className="error" role="alert">
      {what === 'page' ? (
        <>There is no page at this address. Check the address, or go to <Link to="/">Runs</Link>.</>
      ) : (
        <>{`This ${what} does not exist here. It may have been removed.`} Go back to <Link to="/">Runs</Link>.</>
      )}
    </p>
  );
}

/** A load that failed: what was being loaded, the reason, and what to do about it. */
export function LoadFailed({ what, message }: { what: string; message: string }) {
  return (
    <div className="error">
      {`Could not load ${what}:`} <ErrorLine message={message} /> Reload the page to try again.
    </div>
  );
}
