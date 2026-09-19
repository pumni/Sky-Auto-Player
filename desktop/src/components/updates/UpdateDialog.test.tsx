import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { UpdateDialog } from './UpdateDialog';

describe('UpdateDialog', () => {
  afterEach(() => cleanup());

  it('renders explicit checking state when checking for updates', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'checking',
          dialogOpen: true,
          channel: 'stable',
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Checking for updates…' })).toBeInTheDocument();
    expect(
      screen.getByText('Looking for available updates on the stable channel…'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
  });

  it('renders explicit "You\'re up to date" state when current', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'current',
          currentVersion: '4.1.0',
          availableVersion: null,
          channel: 'stable',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: "You're up to date" })).toBeInTheDocument();
    expect(
      screen.getByText('You are running version 4.1.0 on the stable channel.'),
    ).toBeInTheDocument();
    const closeBtn = screen.getByRole('button', { name: 'Close' });
    expect(closeBtn).toBeInTheDocument();

    fireEvent.click(closeBtn);
    expect(useStore.getState().update.dialogOpen).toBe(false);
  });

  it('renders available state with Update, Later, and Skip this version buttons', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'available',
          currentVersion: '4.0.1',
          availableVersion: '4.2.0',
          channel: 'stable',
          releaseNotes: 'Awesome new features.',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Version 4.2.0 is available' })).toBeInTheDocument();
    expect(screen.getByText('You are running 4.0.1 on the stable channel.')).toBeInTheDocument();
    expect(screen.getByText('Awesome new features.')).toBeInTheDocument();

    expect(screen.getByRole('button', { name: /Update and restart/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Later' })).toBeInTheDocument();
    const skipBtn = screen.getByRole('button', { name: 'Skip this version' });
    expect(skipBtn).toBeInTheDocument();

    // Clicking "Skip this version" records skipVersion in settings and closes the dialog
    await act(async () => {
      fireEvent.click(skipBtn);
    });

    expect(useStore.getState().update.dialogOpen).toBe(false);
    expect(useStore.getState().settings?.update_preferences.skip_version).toBe('4.2.0');
  });

  it('renders explicit error state with bounded message and Check again button', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    const checkSpy = vi.spyOn(useStore.getState(), 'checkForUpdate');

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'error',
          errorCode: 'check_failed',
          errorDetail: 'Could not check for updates. Check your network connection and try again.',
          retryAction: 'check',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Update check failed' })).toBeInTheDocument();
    expect(
      screen.getByText('Could not check for updates. Check your network connection and try again.'),
    ).toBeInTheDocument();

    const retryBtn = screen.getByRole('button', { name: 'Check again' });
    expect(retryBtn).toBeInTheDocument();

    fireEvent.click(retryBtn);
    expect(checkSpy).toHaveBeenCalledWith('manual');
  });

  it('renders Try again button when retryAction is install', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    const handoffSpy = vi.spyOn(useStore.getState(), 'beginUpdateHandoff');

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'error',
          errorCode: 'install_failed',
          errorDetail: 'Install process terminated abnormally.',
          retryAction: 'install',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Update failed' })).toBeInTheDocument();
    const retryBtn = screen.getByRole('button', { name: 'Try again' });
    expect(retryBtn).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Check again' })).not.toBeInTheDocument();

    fireEvent.click(retryBtn);
    expect(handoffSpy).toHaveBeenCalledOnce();
  });

  it('renders only Close button when retryAction is none', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'error',
          errorCode: 'channel_unavailable',
          retryAction: 'none',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Update check failed' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Check again' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
  });

  it('surfaces check transportError with Check again button and Update check failed heading', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());
    const checkSpy = vi.spyOn(useStore.getState(), 'checkForUpdate');

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          transportError: 'IPC transport disconnected',
          transportErrorAction: 'check',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Update check failed' })).toBeInTheDocument();
    expect(screen.getByText('IPC transport disconnected')).toBeInTheDocument();
    const retryBtn = screen.getByRole('button', { name: 'Check again' });
    expect(retryBtn).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();

    fireEvent.click(retryBtn);
    expect(checkSpy).toHaveBeenCalledWith('manual');
  });

  it('surfaces install transportError with Try again button and Update failed heading', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());
    const handoffSpy = vi.spyOn(useStore.getState(), 'beginUpdateHandoff');

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          transportError: 'IPC connection failed during install handoff',
          transportErrorAction: 'install',
          dialogOpen: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('heading', { name: 'Update failed' })).toBeInTheDocument();
    expect(screen.getByText('IPC connection failed during install handoff')).toBeInTheDocument();
    const retryBtn = screen.getByRole('button', { name: 'Try again' });
    expect(retryBtn).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Check again' })).not.toBeInTheDocument();

    fireEvent.click(retryBtn);
    expect(handoffSpy).toHaveBeenCalledOnce();
  });

  it('disables buttons when busy installing or checking', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'downloading',
          availableVersion: '4.2.0',
          dialogOpen: true,
          installRequestPending: true,
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByRole('button', { name: 'Close update' })).toBeDisabled();
  });

  it('renders the native cumulative progress snapshot without recalculating it', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'downloading',
          availableVersion: '4.2.0',
          dialogOpen: true,
          progress: {
            completed: 32_000,
            total: 40_000,
            message: 'Downloading update',
          },
        },
      });
    });

    render(<UpdateDialog useStore={useStore} />);

    expect(screen.getByText('Downloading update (32000/40000 bytes)')).toBeInTheDocument();
  });

  it('does not close dialog if skipping version fails to persist', async () => {
    const bridge = createMockBridge();
    bridge.patchSettings = async () => {
      throw new Error('disk full');
    };
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());

    act(() => {
      useStore.setState({
        update: {
          ...useStore.getState().update,
          state: 'available',
          availableVersion: '4.2.0',
          currentVersion: '4.0.1',
          channel: 'stable',
          dialogOpen: true,
        },
      });
    });

    const patchSpy = vi.spyOn(useStore.getState(), 'patchSettings');

    render(<UpdateDialog useStore={useStore} />);
    const skipBtn = screen.getByRole('button', { name: 'Skip this version' });

    await act(async () => {
      fireEvent.click(skipBtn);
    });

    expect(patchSpy).toHaveBeenCalledWith({
      updatePreferences: { skipVersion: '4.2.0' },
    });
    const patchResult = await patchSpy.mock.results[0]?.value;
    expect(patchResult).toBeNull();
    // Dialog stays open because skipVersion failed to persist
    expect(useStore.getState().update.dialogOpen).toBe(true);
  });

  it.each([
    { errorCode: 'stale_update', expectedHeading: 'Update failed', retryAction: 'check' },
    { errorCode: 'update_unavailable', expectedHeading: 'Update failed', retryAction: 'check' },
    { errorCode: 'closing', expectedHeading: 'Update failed', retryAction: 'none' },
    { errorCode: 'check_failed', expectedHeading: 'Update check failed', retryAction: 'check' },
  ] as const)(
    'classifies $errorCode heading as "$expectedHeading"',
    async ({ errorCode, expectedHeading, retryAction }) => {
      const bridge = createMockBridge();
      const useStore = createDesktopStore(bridge);
      await act(async () => useStore.getState().initialize());

      act(() => {
        useStore.setState({
          update: {
            ...useStore.getState().update,
            state: 'error',
            errorCode,
            retryAction,
            dialogOpen: true,
          },
        });
      });

      render(<UpdateDialog useStore={useStore} />);
      expect(screen.getByRole('heading', { name: expectedHeading })).toBeInTheDocument();
    },
  );
});
