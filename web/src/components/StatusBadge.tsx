import { runOutcome } from '../runOutcome';
import type { Tone } from '../statusMap';
import { checkStatusEntry, findingEntry, propertyStatusEntry, replayEntry, workerStateEntry, workerTrustEntry } from '../statusMap';
import type { RunCompletion } from '../types';

export type StatusBadgeProps =
  /** A run. Without `trials` there is no honest scope to give, so the label stands alone. */
  | { readonly kind: 'run'; readonly state: string; readonly trials?: number; readonly completion?: RunCompletion }
  /** A property row's status, as the server sends it. */
  | { readonly kind: 'property'; readonly status: string }
  /** The result of replaying a bundle once. */
  | { readonly kind: 'replay'; readonly failedAgain: boolean }
  /** Where a failure stands, as the server derives it from its stored checks. */
  | { readonly kind: 'finding'; readonly status: string }
  /** One stored check's result. */
  | { readonly kind: 'check'; readonly status: string }
  /** What a worker is doing, as the server works it out. */
  | { readonly kind: 'workerState'; readonly state: string }
  /** Whether an administrator has marked a worker trusted. */
  | { readonly kind: 'workerTrust'; readonly trusted: boolean };

interface BadgeView {
  readonly label: string;
  readonly scope?: string;
  readonly tone: Tone;
  readonly title?: string;
}

function viewOf(props: StatusBadgeProps): BadgeView {
  switch (props.kind) {
    case 'run': {
      const outcome = runOutcome({ state: props.state, trials: props.trials ?? 0, completion: props.completion });
      return props.trials === undefined ? { ...outcome, scope: undefined } : outcome;
    }
    case 'property':
      return { ...propertyStatusEntry(props.status), title: `Server status: ${props.status}` };
    case 'replay':
      return replayEntry(props.failedAgain);
    case 'finding':
      return { ...findingEntry(props.status), title: `Server state: ${props.status}` };
    case 'check':
      return { ...checkStatusEntry(props.status), title: `Server status: ${props.status}` };
    case 'workerState':
      return { ...workerStateEntry(props.state), title: `Server state: ${props.state}` };
    case 'workerTrust':
      return workerTrustEntry(props.trusted);
  }
}

/**
 * The one badge for a run, a property or a replay. The words and tone come from the status table in `statusMap.ts`
 * (and `runOutcome` for a run); the label is always text, the tone adds a colour and a mark in the stylesheet, and the
 * server's own word is in the tooltip so a plainer label never hides it.
 */
export function StatusBadge(props: StatusBadgeProps) {
  const { label, scope, tone, title } = viewOf(props);
  return (
    <>
      <span className={`badge badge-${tone}`} title={title}>
        {label}
      </span>
      {scope !== undefined && (
        <>
          {' '}
          <span className="muted badge-scope">{scope}</span>
        </>
      )}
    </>
  );
}
