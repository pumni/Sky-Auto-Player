import { Component, type ErrorInfo, type PropsWithChildren, type ReactNode } from 'react';
import { isTauriRuntime } from '../platform/runtime';
import { revealStartupWindow } from '../platform/startupWindowReveal';

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<PropsWithChildren, State> {
  public override state: State = { error: null };

  public override componentDidCatch(error: Error, _info: ErrorInfo): void {
    this.setState({ error });
  }

  public override componentDidUpdate(
    _previousProps: PropsWithChildren,
    previousState: State,
  ): void {
    if (!previousState.error && this.state.error) revealStartupWindow('render-error');
  }

  public override render(): ReactNode {
    if (this.state.error) {
      return (
        <main className="fatal-screen" role="alert">
          <p className="eyebrow">DESKTOP UI</p>
          <h1>The desktop surface stopped rendering.</h1>
          <p>{this.state.error.message}</p>
          {isTauriRuntime() ? (
            <p className="muted">Close and reopen the full application to recover.</p>
          ) : (
            <button
              className="button button-primary"
              type="button"
              onClick={() => window.location.reload()}
            >
              Reload window
            </button>
          )}
        </main>
      );
    }
    return this.props.children;
  }
}
