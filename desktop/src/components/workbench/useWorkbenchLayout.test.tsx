import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useWorkbenchLayout, WORKBENCH_STORAGE_KEY } from './useWorkbenchLayout';

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('Inspector topology and preference ownership', () => {
  it('keeps a completed rebased topology across close/reopen until a real viewport change', () => {
    const { result, rerender } = renderHook(
      ({ width, docked }) => useWorkbenchLayout(width, docked),
      { initialProps: { width: 1024, docked: true } },
    );
    act(() => result.current.beginUtilityDrag());
    rerender({ width: 900, docked: true });
    act(() => {
      result.current.setUtilityWidth(264, true);
      result.current.endUtilityDrag();
    });
    expect(result.current.geometry).toMatchObject({
      navigatorWidth: 200,
      trackBrowserWidth: 420,
      utilityWidth: 264,
    });
    rerender({ width: 900, docked: false });
    expect(result.current.utilityWidth).toBe(0);
    rerender({ width: 900, docked: true });
    expect(result.current.geometry).toMatchObject({
      navigatorWidth: 200,
      trackBrowserWidth: 420,
      utilityWidth: 264,
    });
    rerender({ width: 901, docked: true });
    expect(result.current.navigatorCollapsed).toBe(true);
    expect(result.current.utilityWidth).toBe(280);
    expect(JSON.parse(localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      navigatorPreference: 'expanded',
      utilityWidth: 280,
    });
  });
  it.each([
    [900, 264],
    [915, 279],
    [916, 280],
    [917, 281],
  ])('retains effective geometry on release at %s without violating v5', (width, effective) => {
    const { result, rerender } = renderHook(({ width }) => useWorkbenchLayout(width, true), {
      initialProps: { width: 1024 },
    });
    act(() => result.current.beginUtilityDrag());
    rerender({ width });
    expect(result.current.utilityWidth).toBe(effective);
    const geometry = result.current.geometry;
    act(() => {
      result.current.setUtilityWidth(result.current.utilityWidth, true);
      result.current.endUtilityDrag();
    });
    expect(result.current.geometry).toEqual(geometry);
    expect(result.current.navigatorCollapsed).toBe(false);
    expect(JSON.parse(localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      version: 5,
      navigatorPreference: 'expanded',
      utilityWidth: Math.max(280, effective),
    });
    rerender({ width: 1366 });
    expect(result.current.utilityWidth).toBe(Math.max(280, effective));
    expect(result.current.navigatorCollapsed).toBe(false);
  });
  it.each([
    // Independent budget: viewport - 8 padding - 8 separators - 200 navigator - 420 tracks.
    [1024, 388, 388],
    [1100, 320, 440],
    [960, 320, 324],
  ])('locks the expanded navigator at viewport %s', (viewport, initialWidth, maximum) => {
    localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 300,
        utilityWidth: initialWidth,
      }),
    );
    const { result } = renderHook(() => useWorkbenchLayout(viewport, true));
    act(() => result.current.beginUtilityDrag());
    expect(result.current.utilityWidthMax).toBe(maximum);
    for (const requested of [388, 389, 388]) {
      act(() =>
        result.current.setUtilityWidth(Math.min(requested, result.current.utilityWidthMax)),
      );
      expect(result.current.navigatorCollapsed).toBe(false);
      expect(result.current.navigatorWidth).toBeGreaterThanOrEqual(200);
      expect(result.current.geometry.trackBrowserWidth).toBeGreaterThanOrEqual(420);
      if (viewport === 1024) {
        expect(result.current.utilityWidth).toBe(388);
        expect(result.current.navigatorWidth).toBe(200);
        expect(result.current.geometry.trackBrowserWidth).toBe(420);
      }
    }
    act(() => {
      result.current.setUtilityWidth(result.current.utilityWidth, true);
      result.current.endUtilityDrag();
    });
    expect(result.current.navigatorCollapsed).toBe(false);
    expect(JSON.parse(localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 300,
    });
  });
  it('allows an explicit collapsed rail during the session and restores expanded preference after temporary shrink', () => {
    const { result, rerender } = renderHook(
      ({ width, docked }) => useWorkbenchLayout(width, docked),
      { initialProps: { width: 1024, docked: true } },
    );
    act(() => result.current.beginUtilityDrag());
    act(() => result.current.setNavigatorPreference('collapsed', true));
    expect(result.current.navigatorWidth).toBe(48);
    expect(result.current.utilityWidthMax).toBe(440);
    act(() => {
      result.current.setUtilityWidth(440, true);
      result.current.endUtilityDrag();
    });
    expect(result.current.geometry.trackBrowserWidth).toBe(520);
    act(() => {
      result.current.setUtilityWidth(320);
      result.current.setNavigatorPreference('expanded', true);
    });
    const saved = localStorage.getItem(WORKBENCH_STORAGE_KEY);
    rerender({ width: 640, docked: false });
    expect(result.current.navigatorCollapsed).toBe(true);
    expect(result.current.navigatorPreference).toBe('expanded');
    expect(localStorage.getItem(WORKBENCH_STORAGE_KEY)).toBe(saved);
    rerender({ width: 1024, docked: true });
    expect(result.current.navigatorCollapsed).toBe(false);
  });
  it('adapts a locked session to viewport bounds and discards the lock when separator unmounts', () => {
    const { result, rerender } = renderHook(
      ({ width, docked }) => useWorkbenchLayout(width, docked),
      { initialProps: { width: 1024, docked: true } },
    );
    act(() => result.current.beginUtilityDrag());
    rerender({ width: 900, docked: true });
    expect(result.current.utilityWidth).toBe(264);
    expect(result.current.navigatorWidth).toBe(200);
    expect(result.current.geometry.trackBrowserWidth).toBe(420);
    rerender({ width: 640, docked: false });
    expect(result.current.navigatorWidth).toBe(48);
    rerender({ width: 800, docked: true });
    expect(result.current.navigatorWidth).toBe(48);
    expect(result.current.utilityWidth).toBe(316);
  });
  it('makes identical queued patches no-ops while still persisting the final width once', () => {
    let renders = 0;
    const { result } = renderHook(() => {
      renders++;
      return useWorkbenchLayout(1200, true);
    });
    const writes = vi.spyOn(Storage.prototype, 'setItem');
    const before = renders;
    act(() => {
      result.current.setUtilityWidth(320);
      result.current.setUtilityWidth(320);
    });
    expect(renders).toBe(before);
    act(() => {
      result.current.setUtilityWidth(360);
      result.current.setUtilityWidth(360);
    });
    expect(renders).toBe(before + 1);
    expect(writes).not.toHaveBeenCalled();
    act(() => result.current.setUtilityWidth(360, true));
    expect(renders).toBe(before + 1);
    expect(writes).toHaveBeenCalledTimes(1);
  });
});

describe('shared palette geometry boundary', () => {
  it('keeps palette selectors color-only and the solver palette-independent', () => {
    const solver = readFileSync(
      resolve(process.cwd(), 'src/components/workbench/useWorkbenchLayout.ts'),
      'utf8',
    );
    expect(solver).not.toMatch(/palette|PaletteId/);

    const themes = readFileSync(resolve(process.cwd(), 'src/styles/themes.css'), 'utf8');
    const paletteRules = [...themes.matchAll(/([^{}]*data-palette[^{}]*)\{([^{}]*)\}/g)];
    expect(paletteRules).toHaveLength(6);
    for (const paletteRule of paletteRules) {
      const selector = paletteRule[1];
      const declarations = paletteRule[2];
      if (selector === undefined || declarations === undefined) {
        throw new Error('Palette rule is missing its selector or declarations');
      }
      expect(selector).not.toMatch(
        /(?:min|max)?(?:width|height)|gap|padding|margin|radius|font|transition/i,
      );
      for (const declaration of declarations
        .split(';')
        .map((part) => part.trim())
        .filter(Boolean)) {
        expect(declaration).toMatch(/^--[\w-]+\s*:\s*[^{}]+$/);
        expect(declaration).not.toMatch(
          /(?:min|max)?(?:width|height)|gap|padding|margin|radius|font|transition/i,
        );
      }
    }
  });
});
