import { Component, type ReactNode } from 'react';

export function ErrorNotice({ label, message }: { label: string; message: string }) {
  return (
    <p className="error" role="alert">
      {`${label} could not be shown: ${message}`}
    </p>
  );
}

interface Props {
  label: string;
  children: ReactNode;
}

interface State {
  message: string | undefined;
}

/**
 * Stops a response that is valid JSON of an unexpected shape from blanking the whole page: the part that cannot render
 * shows an error, and the rest of the page stays. Only the message is shown, never a component stack.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { message: undefined };

  static getDerivedStateFromError(error: unknown): State {
    return { message: error instanceof Error ? error.message : String(error) };
  }

  render() {
    if (this.state.message !== undefined) return <ErrorNotice label={this.props.label} message={this.state.message} />;
    return this.props.children;
  }
}
