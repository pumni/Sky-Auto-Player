import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ResizableSeparator } from './ResizableSeparator';
import {
  DEFAULT_NAVIGATOR_WIDTH,
  COMPACT_NAVIGATOR_WIDTH,
  getNavigatorWidthMax,
  LEGACY_WORKBENCH_STORAGE_KEY,
  LEGACY_WORKBENCH_V1_STORAGE_KEY,
  loadWorkbenchLayout,
  MIN_NAVIGATOR_WIDTH,
  getUtilityWidthMax,
  solveWorkbenchGeometry,
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

  it('loads valid v3 values and clamps them to current bounds', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 3,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 640,
        utilityWidth: 340,
      }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 3,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 360,
      utilityWidth: 340,
    });
  });

  it('soft-migrates valid v2 geometry without persisting presentation state', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 2, navigatorWidth: 280, utilityWidth: 490 }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 3,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 280,
      utilityWidth: 480,
    });
  });

  it('soft-migrates v1 library geometry', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_V1_STORAGE_KEY,
      JSON.stringify({ version: 1, libraryWidth: 280, utilityWidth: 490, utilityOpen: true }),
    );
    expect(loadWorkbenchLayout().expandedNavigatorWidth).toBe(280);
  });

  it('falls back to defaults for malformed or wrong-version values', () => {
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, '{not json');
    expect(loadWorkbenchLayout().expandedNavigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify({ version: 3 }));
    window.localStorage.removeItem(LEGACY_WORKBENCH_STORAGE_KEY);
    expect(loadWorkbenchLayout().expandedNavigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    expect(loadWorkbenchLayout().utilityWidth).toBe(360);
  });

  it('accounts for padding, separators, and the minimum track browser width', () => {
    expect(getNavigatorWidthMax(900)).toBe(360);
    expect(getNavigatorWidthMax(1200, 340)).toBe(348);
    expect(getNavigatorWidthMax(920, 320)).toBe(MIN_NAVIGATOR_WIDTH);
    expect(getUtilityWidthMax(920)).toBe(336);
    const minimum = solveWorkbenchGeometry({
      viewportWidth: 920,
      navigatorWidth: COMPACT_NAVIGATOR_WIDTH,
      utilityWidth: getUtilityWidthMax(920),
    });
    expect(minimum.trackBrowserWidth).toBe(480);
    expect(minimum.fits).toBe(true);
    expect(COMPACT_NAVIGATOR_WIDTH).toBe(72);
    expect(getNavigatorWidthMax(920)).toBeGreaterThanOrEqual(MIN_NAVIGATOR_WIDTH);
  });
});
