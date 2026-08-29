import { Component, type ErrorInfo, type PropsWithChildren, type ReactNode } from 'react';

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<PropsWithChildren, State> {
  public override state: State = { error: null };

  public override componentDidCatch(error: Error, _info: ErrorInfo): void {
    this.setState({ error });
  }

  public override render(): ReactNode {
    if (this.state.error) {
      return (
        <main className="fatal-screen" role="alert">
          <p className="eyebrow">DESKTOP UI</p>
          <h1>The desktop surface stopped rendering.</h1>
          <p>{this.state.error.message}</p>
          <button
            className="button button-primary"
            type="button"
            onClick={() => window.location.reload()}
          >
            Reload window
          </button>
        </main>
      );
    }
    return this.props.children;
  }
}
