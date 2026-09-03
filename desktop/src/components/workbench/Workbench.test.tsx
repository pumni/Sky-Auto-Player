import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ResizableSeparator } from './ResizableSeparator';
import {
  DEFAULT_NAVIGATOR_WIDTH,
  getNavigatorWidthMax,
  LEGACY_WORKBENCH_STORAGE_KEY,
  loadWorkbenchLayout,
  MIN_NAVIGATOR_WIDTH,
  WORKBENCH_STORAGE_KEY,
} from './useWorkbenchLayout';

describe('ResizableSeparator', () => {
  afterEach(() => window.localStorage.clear());

  it('supports keyboard resize, clamping, and reset', () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResizableSeparator
        label="Resize library navigator"
        value={260}
        min={220}
        max={360}
        defaultValue={260}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole('separator', { name: 'Resize library navigator' });

    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(268);
    expect(onCommit).toHaveBeenLastCalledWith(268);

    rerender(
      <ResizableSeparator
        label="Resize library navigator"
        value={268}
        min={220}
        max={360}
        defaultValue={260}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'ArrowLeft',
      shiftKey: true,
    });
    expect(onChange).toHaveBeenLastCalledWith(236);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'Home',
    });
    expect(onChange).toHaveBeenLastCalledWith(220);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'End',
    });
    expect(onChange).toHaveBeenLastCalledWith(360);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'Enter',
    });
    expect(onChange).toHaveBeenLastCalledWith(DEFAULT_NAVIGATOR_WIDTH);
  });

  it('reverses pointer semantics for a pane owned after the separator', () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResizableSeparator
        label="Resize utility pane"
        value={360}
        min={320}
        max={480}
        defaultValue={360}
        direction={-1}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole('separator', { name: 'Resize utility pane' });

    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(352);
    rerender(
      <ResizableSeparator
        label="Resize utility pane"
        value={352}
        min={320}
        max={480}
        defaultValue={360}
        direction={-1}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    fireEvent.keyDown(separator, { key: 'ArrowLeft', shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(384);
    expect(onCommit).toHaveBeenLastCalledWith(384);
  });
});

describe('workbench layout persistence', () => {
  afterEach(() => window.localStorage.clear());

  it('loads valid v2 values and clamps them to current bounds', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 2, navigatorWidth: 640, utilityWidth: 340 }),
    );
    expect(loadWorkbenchLayout(400)).toEqual({
      version: 2,
      navigatorWidth: 400,
      utilityWidth: 340,
    });
  });

  it('soft-migrates valid v1 geometry without persisting presentation state', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 1, libraryWidth: 280, utilityWidth: 490, utilityOpen: true }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 2,
      navigatorWidth: 280,
      utilityWidth: 480,
    });
  });

  it('falls back to defaults for malformed or wrong-version values', () => {
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, '{not json');
    expect(loadWorkbenchLayout().navigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify({ version: 3 }));
    window.localStorage.removeItem(LEGACY_WORKBENCH_STORAGE_KEY);
    expect(loadWorkbenchLayout().navigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    expect(loadWorkbenchLayout().utilityWidth).toBe(360);
  });

  it('accounts for padding, separators, and the minimum track browser width', () => {
    expect(getNavigatorWidthMax(900)).toBe(360);
    expect(getNavigatorWidthMax(1200, 340)).toBe(340);
    expect(getNavigatorWidthMax(920)).toBeGreaterThanOrEqual(MIN_NAVIGATOR_WIDTH);
  });
});
