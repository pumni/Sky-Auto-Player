import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { WindowControls } from '../../platform/windowControls';

type NativeHoverEvent = { payload: { hovered: boolean } };
type NativeHoverListener = (event: NativeHoverEvent) => void;
type NativeWindowListener = (event: string, listener: NativeHoverListener) => Promise<() => void>;

const mocks = vi.hoisted(() => ({
  isTauriRuntime: vi.fn<() => boolean>(() => false),
  listen: vi.fn<NativeWindowListener>(),
  unlisten: vi.fn<() => void>(),
}));

vi.mock('../../platform/runtime', () => ({ isTauriRuntime: mocks.isTauriRuntime }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ listen: mocks.listen }),
}));

import { WindowCaptionControls } from './WindowCaptionControls';

function createControls(): WindowControls {
  return {
    minimize: vi.fn(async () => undefined),
    toggleMaximize: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
    isMaximized: vi.fn(async () => false),
    onResize: vi.fn(async () => () => undefined),
  };
}

describe('WindowCaptionControls native hover bridge', () => {
  let nativeListener: NativeHoverListener | undefined;

  beforeEach(() => {
    nativeListener = undefined;
    mocks.isTauriRuntime.mockReturnValue(false);
    mocks.listen.mockReset();
    mocks.unlisten.mockReset();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('uses browser CSS hover without subscribing to native events', () => {
    render(<WindowCaptionControls controls={createControls()} />);

    expect(mocks.listen).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Maximize window' })).not.toHaveAttribute(
      'data-native-hover',
    );
  });

  it('applies native hover only to Maximize and clears it on leave and window blur', async () => {
    mocks.isTauriRuntime.mockReturnValue(true);
    mocks.listen.mockImplementation(async (_event, listener) => {
      nativeListener = listener;
      return mocks.unlisten;
    });
    const { unmount } = render(<WindowCaptionControls controls={createControls()} />);

    await waitFor(() =>
      expect(mocks.listen).toHaveBeenCalledWith('sky:caption-zoom-hover', expect.any(Function)),
    );
    const maximize = screen.getByRole('button', { name: 'Maximize window' });
    const minimize = screen.getByRole('button', { name: 'Minimize window' });
    const close = screen.getByRole('button', { name: 'Close window' });

    act(() => nativeListener?.({ payload: { hovered: true } }));
    expect(maximize).toHaveAttribute('data-native-hover', 'true');
    expect(minimize).not.toHaveAttribute('data-native-hover');
    expect(close).not.toHaveAttribute('data-native-hover');

    fireEvent.blur(window);
    expect(maximize).not.toHaveAttribute('data-native-hover');
    act(() => nativeListener?.({ payload: { hovered: true } }));
    expect(maximize).not.toHaveAttribute('data-native-hover');

    fireEvent.focus(window);
    act(() => nativeListener?.({ payload: { hovered: true } }));
    expect(maximize).toHaveAttribute('data-native-hover', 'true');
    act(() => nativeListener?.({ payload: { hovered: false } }));
    expect(maximize).not.toHaveAttribute('data-native-hover');

    unmount();
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it('unlistens if native listener registration finishes after unmount', async () => {
    mocks.isTauriRuntime.mockReturnValue(true);
    let completeRegistration: ((unlisten: () => void) => void) | undefined;
    mocks.listen.mockImplementation(
      () => new Promise((resolve) => (completeRegistration = resolve)),
    );
    const { unmount } = render(<WindowCaptionControls controls={createControls()} />);

    await waitFor(() => expect(mocks.listen).toHaveBeenCalledOnce());
    unmount();
    const unlisten = vi.fn();
    await act(async () => completeRegistration?.(unlisten));

    expect(unlisten).toHaveBeenCalledOnce();
  });
});
