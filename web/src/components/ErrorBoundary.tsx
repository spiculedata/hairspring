import { Component, Fragment, type ReactNode } from 'react';

export function ErrorNotice({ label, message, onRetry }: { label: string; message: string; onRetry?: () => void }) {
  return (
    <div className="error-notice">
      <p className="error" role="alert">
        {`${label} could not be shown: ${message}`}
      </p>
      {onRetry && (
        <button type="button" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  );
}

interface Props {
  label: string;
  children: ReactNode;
  /** Loads the data again. Needed when the data the part failed on is held by its owner, outside the boundary. */
  onRetry?: () => void;
}

interface State {
  message: string | undefined;
  /** Counts retries; it keys the children, so a retry mounts them afresh and what failed loads again. */
  attempt: number;
}

/** The state after Retry: no failure, and a new `attempt` so the children are mounted again. */
export const afterRetry = (state: State): State => ({ message: undefined, attempt: state.attempt + 1 });

/**
 * Stops a response that is valid JSON of an unexpected shape from blanking the whole page: the part that cannot render
 * shows an error, and the rest of the page stays. Only the message is shown, never a component stack. Retry mounts the
 * part again, and `onRetry` has the owner load again the data the part was given, so a failure that has passed (a fixed
 * server, a restored connection) clears.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { message: undefined, attempt: 0 };

  static getDerivedStateFromError(error: unknown): Pick<State, 'message'> {
    return { message: error instanceof Error ? error.message : String(error) };
  }

  retry = (): void => {
    this.props.onRetry?.();
    this.setState(afterRetry);
  };

  render() {
    if (this.state.message !== undefined) return <ErrorNotice label={this.props.label} message={this.state.message} onRetry={this.retry} />;
    return <Fragment key={this.state.attempt}>{this.props.children}</Fragment>;
  }
}
