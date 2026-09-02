import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ResizableSeparator } from './ResizableSeparator';
import {
  DEFAULT_LIBRARY_WIDTH,
  loadWorkbenchLayout,
  WORKBENCH_STORAGE_KEY,
} from './useWorkbenchLayout';

describe('ResizableSeparator', () => {
  afterEach(() => window.localStorage.clear());

  it('supports keyboard resize, clamping, and reset', () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResizableSeparator
        label="Resize library pane"
        value={344}
        min={280}
        max={520}
        defaultValue={344}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole('separator', { name: 'Resize library pane' });

    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(352);
    expect(onCommit).toHaveBeenLastCalledWith(352);

    rerender(
      <ResizableSeparator
        label="Resize library pane"
        value={352}
        min={280}
        max={520}
        defaultValue={344}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library pane' }), {
      key: 'ArrowLeft',
      shiftKey: true,
    });
    expect(onChange).toHaveBeenLastCalledWith(320);

    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library pane' }), {
      key: 'Home',
    });
    expect(onChange).toHaveBeenLastCalledWith(280);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library pane' }), {
      key: 'End',
    });
    expect(onChange).toHaveBeenLastCalledWith(520);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library pane' }), {
      key: 'Enter',
    });
    expect(onChange).toHaveBeenLastCalledWith(DEFAULT_LIBRARY_WIDTH);
  });
});

describe('workbench layout persistence', () => {
  afterEach(() => window.localStorage.clear());

  it('loads valid values and clamps them to the current bounds', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 1, libraryWidth: 640, utilityWidth: 340, utilityOpen: true }),
    );
    expect(loadWorkbenchLayout(400)).toMatchObject({
      version: 1,
      libraryWidth: 400,
      utilityWidth: 340,
      utilityOpen: true,
    });
  });

  it('falls back to defaults for malformed or wrong-version values', () => {
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, '{not json');
    expect(loadWorkbenchLayout().libraryWidth).toBe(DEFAULT_LIBRARY_WIDTH);
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 2, libraryWidth: 280 }),
    );
    expect(loadWorkbenchLayout().libraryWidth).toBe(DEFAULT_LIBRARY_WIDTH);
  });
});
