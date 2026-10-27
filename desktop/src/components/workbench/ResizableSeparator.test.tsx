import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ResizableSeparator } from './ResizableSeparator';

describe('separator drag sessions', () => {
  let frames: Map<number, FrameRequestCallback>;
  let id = 0;
  beforeEach(() => {
    frames = new Map();
    vi.stubGlobal(
      'PointerEvent',
      class extends MouseEvent {
        pointerId: number;
        constructor(type: string, init: PointerEventInit = {}) {
          super(type, init);
          this.pointerId = init.pointerId ?? 1;
        }
      },
    );
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
      frames.set(++id, callback);
      return id;
    });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((frame) => {
      frames.delete(frame);
    });
    Element.prototype.setPointerCapture = vi.fn();
    Element.prototype.hasPointerCapture = vi.fn(() => true);
    Element.prototype.releasePointerCapture = vi.fn();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });
  function frame() {
    act(() => {
      const pending = [...frames.values()];
      frames.clear();
      pending.forEach((callback) => callback(0));
    });
  }
  function setup(overrides = {}) {
    const props = {
      label: 'Resize',
      value: 320,
      min: 280,
      max: 388,
      defaultValue: 320,
      direction: -1 as const,
      onChange: vi.fn(),
      onCommit: vi.fn(),
      onDragStart: vi.fn(),
      onDragEnd: vi.fn(),
      ...overrides,
    };
    const view = render(<ResizableSeparator {...props} />);
    const separator = screen.getByRole('separator');
    const move = (x: number) => fireEvent.pointerMove(separator, { clientX: x });
    fireEvent.pointerDown(separator, { button: 0, clientX: 500 });
    return { ...view, props, separator, move };
  }
  it('coalesces a burst into its last value and deduplicates repeated clamped moves', () => {
    const { props, move } = setup();
    for (let x = 499; x >= 400; x--) move(x);
    expect(props.onChange).not.toHaveBeenCalled();
    frame();
    expect(props.onChange.mock.calls).toEqual([[388]]);
    for (let x = 399; x >= 300; x--) {
      move(x);
      frame();
    }
    expect(props.onChange).toHaveBeenCalledTimes(1);
    expect(props.onCommit).not.toHaveBeenCalled();
  });
  it('records a deterministic drag update proxy', () => {
    const { props, separator, move } = setup();
    for (let x = 499; x >= 400; x--) move(x);
    frame();
    for (let x = 399; x >= 300; x--) {
      move(x);
      frame();
    }
    fireEvent.pointerUp(separator);
    console.info(
      JSON.stringify({
        proxy: 'jsdom drag callbacks, not Windows FPS',
        pointerMoves: 200,
        changes: props.onChange.mock.calls.length,
        commits: props.onCommit.mock.calls.length,
        finalWidth: props.onCommit.mock.lastCall?.[0],
      }),
    );
    expect(props.onChange).toHaveBeenCalledTimes(1);
    expect(props.onCommit.mock.calls).toEqual([[388]]);
  });
  it.each(['pointerUp', 'pointerCancel', 'lostPointerCapture'] as const)(
    'flushes and commits once on %s despite subsequent end events',
    (end) => {
      const { props, separator, move } = setup();
      move(470);
      fireEvent[end](separator);
      fireEvent.lostPointerCapture(separator);
      fireEvent.pointerUp(separator);
      frame();
      expect(props.onChange.mock.calls).toEqual([[350]]);
      expect(props.onCommit.mock.calls).toEqual([[350]]);
      expect(props.onDragEnd).toHaveBeenCalledTimes(1);
      expect(document.body).not.toHaveClass('is-resizing');
    },
  );
  it('rebases pending and already applied widths to changed viewport bounds', () => {
    const { props, rerender, separator, move } = setup();
    move(440);
    frame();
    expect(props.onChange).toHaveBeenLastCalledWith(380);
    rerender(<ResizableSeparator {...props} value={380} max={340} />);
    frame();
    expect(props.onChange).toHaveBeenLastCalledWith(340);
    move(420);
    rerender(<ResizableSeparator {...props} value={340} max={330} />);
    fireEvent.pointerUp(separator);
    expect(props.onCommit.mock.calls).toEqual([[330]]);
  });
  it('unmount cancels pending work without callbacks or commit', () => {
    const { props, move, unmount } = setup();
    move(440);
    const staleCallback = [...frames.values()][0]!;
    expect(document.body).toHaveClass('is-resizing');
    unmount();
    frame();
    act(() => staleCallback(0));
    expect(frames.size).toBe(0);
    expect(props.onChange).not.toHaveBeenCalled();
    expect(props.onCommit).not.toHaveBeenCalled();
    expect(props.onDragEnd).not.toHaveBeenCalled();
    expect(document.body).not.toHaveClass('is-resizing');
  });
  it('ignores other pointers and keeps capture until the owning pointer finishes', () => {
    const { props, separator, move } = setup();
    fireEvent.pointerDown(separator, { button: 0, clientX: 800, pointerId: 2 });
    fireEvent.pointerMove(separator, { clientX: 0, pointerId: 2 });
    fireEvent.pointerCancel(separator, { pointerId: 2 });
    fireEvent.lostPointerCapture(separator, { pointerId: 2 });
    fireEvent.pointerUp(separator, { pointerId: 2 });
    expect(props.onDragStart).toHaveBeenCalledTimes(1);
    expect(props.onCommit).not.toHaveBeenCalled();
    move(470);
    fireEvent.pointerUp(separator);
    expect(props.onCommit.mock.calls).toEqual([[350]]);
    expect(separator.releasePointerCapture).toHaveBeenCalledWith(1);
  });
  it.each([1, -1] as const)(
    'preserves keyboard/reset/disabled controls in direction %s',
    (direction) => {
      const { props, separator, rerender } = setup({ direction, defaultValue: 500 });
      fireEvent.pointerUp(separator);
      props.onCommit.mockClear();
      for (const [key, shiftKey, expected] of [
        ['ArrowRight', false, 320 + direction * 8],
        ['ArrowLeft', true, 320 - direction * 32],
        ['Home', false, 280],
        ['End', false, 388],
        ['Enter', false, 388],
      ] as const) {
        fireEvent.keyDown(separator, { key, shiftKey });
        expect(props.onCommit).toHaveBeenLastCalledWith(expected);
      }
      fireEvent.doubleClick(separator);
      expect(props.onCommit).toHaveBeenLastCalledWith(388);
      rerender(<ResizableSeparator {...props} disabled />);
      props.onChange.mockClear();
      props.onCommit.mockClear();
      fireEvent.keyDown(separator, { key: 'End' });
      fireEvent.doubleClick(separator);
      fireEvent.pointerDown(separator, { button: 0, clientX: 500 });
      expect(separator).toHaveAttribute('tabindex', '-1');
      expect(props.onChange).not.toHaveBeenCalled();
      expect(props.onCommit).not.toHaveBeenCalled();
    },
  );
});
