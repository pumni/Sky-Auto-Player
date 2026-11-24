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
        <main
          className="grid min-h-dvh content-center justify-items-center gap-2 bg-background p-6 text-center"
          role="alert"
        >
          <p className="m-0 text-[11px] font-semibold tracking-wide text-danger">DESKTOP UI</p>
          <h1 className="m-0 max-w-[580px] text-xl font-semibold leading-7 text-foreground">
            The desktop surface stopped rendering.
          </h1>
          <p className="m-0 max-w-[580px] text-sm text-text-secondary">
            {this.state.error.message}
          </p>
          {isTauriRuntime() ? (
            <p className="m-0 text-sm text-muted-foreground">
              Close and reopen the full application to recover.
            </p>
          ) : (
            <button
              className="h-9 rounded-control bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
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
