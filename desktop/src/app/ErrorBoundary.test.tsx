import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { isTauriRuntime } from '../platform/runtime';
import { ErrorBoundary } from './ErrorBoundary';

vi.mock('../platform/runtime', () => ({
  isTauriRuntime: vi.fn(),
}));

function ThrowOnRender(): never {
  throw new Error('render failure');
}

describe('ErrorBoundary recovery', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders children when they do not throw', () => {
    render(
      <ErrorBoundary>
        <p>Healthy child</p>
      </ErrorBoundary>,
    );

    expect(screen.getByText('Healthy child')).toBeInTheDocument();
  });

  it('instructs Tauri users to reopen the full application without offering a page reload', () => {
    vi.mocked(isTauriRuntime).mockReturnValue(true);
    vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <ErrorBoundary>
        <ThrowOnRender />
      </ErrorBoundary>,
    );

    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: 'The desktop surface stopped rendering.' }),
    ).toBeInTheDocument();
    expect(screen.getByText('render failure')).toBeInTheDocument();
    expect(
      screen.getByText('Close and reopen the full application to recover.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Reload window' })).not.toBeInTheDocument();
  });

  it('retains the page reload action for browser users', () => {
    vi.mocked(isTauriRuntime).mockReturnValue(false);
    vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <ErrorBoundary>
        <ThrowOnRender />
      </ErrorBoundary>,
    );

    expect(screen.getByRole('alert')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reload window' })).toBeInTheDocument();
  });
});
