import { useConnectionLoss } from './useConnectionLoss';

const MESSAGES = {
  offline: 'You are offline. The data shown is the last data received. This page refreshes when the connection returns.',
  unreachable: 'Cannot reach the server. The data shown is the last data received. This page tries again by itself.',
} as const;

/**
 * A banner for when the page cannot get fresh data: the browser says it is offline, or requests have failed several times
 * in a row. It says the data shown is the last that arrived and that the page tries again by itself. Mount it once, in the
 * page chrome; otherwise it renders an empty live region.
 */
export function ConnectionBanner() {
  const loss = useConnectionLoss();
  const message = loss === undefined ? undefined : MESSAGES[loss];
  // The live region stays in the page so that screen readers register it before the text is added to it.
  return (
    <div className="connection-status" role="status">
      {message !== undefined && <div className="connection-banner">{message}</div>}
    </div>
  );
}
