import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import type { WindowControls } from '../../platform/windowControls';
import { AppTitleBar } from './AppTitleBar';

async function testBootstrap(): Promise<{
  bootstrap: Bootstrap;
  useStore: ReturnType<typeof createDesktopStore>;
}> {
  const bridge = createMockBridge();
  const useStore = createDesktopStore(bridge);
  await act(async () => useStore.getState().initialize());
  const bootstrap = useStore.getState().bootstrap;
  if (!bootstrap) throw new Error('mock bootstrap did not load');
  return { bootstrap, useStore };
}

describe('AppTitleBar', () => {
  afterEach(() => cleanup());

  it('exposes caption actions, search, and no version label', async () => {
    const { bootstrap, useStore } = await testBootstrap();
    const minimize = vi.fn(async () => undefined);
    const toggleMaximize = vi.fn(async () => undefined);
    const close = vi.fn(async () => undefined);
    const controls: WindowControls = {
      minimize,
      toggleMaximize,
      close,
      isMaximized: vi.fn(async () => false),
      onResize: vi.fn(async () => () => undefined),
    };
    render(<AppTitleBar bootstrap={bootstrap} useStore={useStore} windowControls={controls} />);

    expect(screen.getByPlaceholderText('Search library…')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Minimize window' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Maximize window' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close window' })).toBeInTheDocument();
    expect(screen.queryByText(`v${bootstrap.app_version}`)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Minimize window' }));
    fireEvent.click(screen.getByRole('button', { name: 'Maximize window' }));
    fireEvent.click(screen.getByRole('button', { name: 'Close window' }));
    expect(minimize).toHaveBeenCalledOnce();
    expect(toggleMaximize).toHaveBeenCalledOnce();
    expect(close).toHaveBeenCalledOnce();
  });

  it('refreshes the maximize state after resize', async () => {
    const { bootstrap, useStore } = await testBootstrap();
    let maximized = false;
    let onResize: (() => void) | undefined;
    const controls: WindowControls = {
      minimize: async () => undefined,
      toggleMaximize: async () => {
        maximized = true;
      },
      close: async () => undefined,
      isMaximized: async () => maximized,
      onResize: async (listener) => {
        onResize = listener;
        return () => undefined;
      },
    };
    render(<AppTitleBar bootstrap={bootstrap} useStore={useStore} windowControls={controls} />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Maximize window' })).toBeVisible(),
    );

    await act(async () => {
      await controls.toggleMaximize();
      onResize?.();
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Restore window' })).toBeVisible(),
    );

    await act(async () => {
      maximized = false;
      onResize?.();
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Maximize window' })).toBeVisible(),
    );
  });

  it('settles a delayed restore state after the first resize query', async () => {
    const { bootstrap, useStore } = await testBootstrap();
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    });
    let onResize: (() => void) | undefined;
    const isMaximized = vi
      .fn<() => Promise<boolean>>()
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(false);
    const controls: WindowControls = {
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      isMaximized,
      onResize: async (listener) => {
        onResize = listener;
        return () => undefined;
      },
    };

    render(<AppTitleBar bootstrap={bootstrap} useStore={useStore} windowControls={controls} />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Maximize window' })).toBeVisible(),
    );

    await act(async () => {
      onResize?.();
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Restore window' })).toBeVisible(),
    );

    await act(async () => {
      frames.shift()?.(0);
      frames.shift()?.(0);
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Maximize window' })).toBeVisible(),
    );
    expect(isMaximized).toHaveBeenCalledTimes(3);
    vi.unstubAllGlobals();
  });
});
