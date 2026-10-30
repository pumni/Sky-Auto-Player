import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { WORKBENCH_STORAGE_KEY } from '../../state/workbenchPreferences';
import { useWorkbenchPreferences } from './useWorkbenchPreferences';

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('workbench preference persistence', () => {
  it('keeps live geometry transient and persists only an explicit final commit', () => {
    const { result } = renderHook(() => useWorkbenchPreferences());
    const write = vi.spyOn(Storage.prototype, 'setItem');

    act(() => {
      result.current.update({ utilityWidth: 352 });
      result.current.update({ expandedNavigatorWidth: 280 });
    });

    expect(result.current.layout).toMatchObject({ utilityWidth: 352, expandedNavigatorWidth: 280 });
    expect(write).not.toHaveBeenCalled();

    act(() => result.current.update({ utilityWidth: 360 }, true));

    expect(write).toHaveBeenCalledTimes(1);
    expect(JSON.parse(localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 280,
      utilityWidth: 360,
    });
  });
});
