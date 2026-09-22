import { useRef, useState } from 'react';
import {
  COMPACT_NAVIGATOR_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  DEFAULT_UTILITY_WIDTH,
  LEGACY_WORKBENCH_STORAGE_KEY,
  LEGACY_WORKBENCH_V1_STORAGE_KEY,
  LEGACY_WORKBENCH_V2_STORAGE_KEY,
  LEGACY_WORKBENCH_V3_STORAGE_KEY,
  LEGACY_WORKBENCH_V4_STORAGE_KEY,
  MAX_NAVIGATOR_WIDTH,
  MAX_UTILITY_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
  WORKBENCH_STORAGE_KEY,
  loadWorkbenchPreferences,
  normalizeWorkbenchPreferences,
  updateWorkbenchPreferences,
} from '../../state/workbenchPreferences';
import type {
  NavigatorPreference,
  WorkbenchPreferencePatch,
  WorkbenchPreferencesV5,
} from '../../state/workbenchPreferences';

export {
  COMPACT_NAVIGATOR_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  DEFAULT_UTILITY_WIDTH,
  LEGACY_WORKBENCH_STORAGE_KEY,
  LEGACY_WORKBENCH_V1_STORAGE_KEY,
  LEGACY_WORKBENCH_V2_STORAGE_KEY,
  LEGACY_WORKBENCH_V3_STORAGE_KEY,
  LEGACY_WORKBENCH_V4_STORAGE_KEY,
  MAX_NAVIGATOR_WIDTH,
  MAX_UTILITY_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
  WORKBENCH_STORAGE_KEY,
  loadWorkbenchPreferences as loadWorkbenchLayout,
  normalizeWorkbenchPreferences,
  updateWorkbenchPreferences,
};
export type { NavigatorPreference, WorkbenchPreferencesV5 };
export type WorkbenchLayoutStateV4 = WorkbenchPreferencesV5;

export const MIN_TRACK_BROWSER_WIDTH = 420;
export const WORKBENCH_GUTTER = 8;
export const OUTER_INLINE_PADDING = WORKBENCH_GUTTER * 2;

interface WorkbenchGeometryInput {
  viewportWidth: number;
  navigatorWidth: number;
  utilityWidth: number;
}

export interface WorkbenchGeometry {
  availableWidth: number;
  outerInlinePadding: number;
  separatorSpace: number;
  navigatorWidth: number;
  trackBrowserWidth: number;
  utilityWidth: number;
  fits: boolean;
}

function availableNavigatorWidth(viewportWidth: number, utilityWidth: number): number {
  const separatorSpace = utilityWidth > 0 ? WORKBENCH_GUTTER * 2 : WORKBENCH_GUTTER;
  return Math.floor(
    viewportWidth - OUTER_INLINE_PADDING - separatorSpace - utilityWidth - MIN_TRACK_BROWSER_WIDTH,
  );
}

export function getNavigatorWidthMax(viewportWidth: number, utilityWidth = 0): number {
  return Math.max(
    MIN_NAVIGATOR_WIDTH,
    Math.min(MAX_NAVIGATOR_WIDTH, availableNavigatorWidth(viewportWidth, utilityWidth)),
  );
}

export function getUtilityWidthMax(viewportWidth: number): number {
  const available =
    viewportWidth -
    OUTER_INLINE_PADDING -
    WORKBENCH_GUTTER * 2 -
    COMPACT_NAVIGATOR_WIDTH -
    MIN_TRACK_BROWSER_WIDTH;
  return Math.min(MAX_UTILITY_WIDTH, Math.max(0, Math.floor(available)));
}

export function getUtilityWidthMin(viewportWidth: number): number {
  return Math.min(MIN_UTILITY_WIDTH, getUtilityWidthMax(viewportWidth));
}

export function solveWorkbenchGeometry({
  viewportWidth,
  navigatorWidth,
  utilityWidth,
}: WorkbenchGeometryInput): WorkbenchGeometry {
  const availableWidth = Math.max(0, Math.floor(viewportWidth));
  const separatorSpace = utilityWidth > 0 ? WORKBENCH_GUTTER * 2 : WORKBENCH_GUTTER;
  const trackBrowserWidth =
    availableWidth - OUTER_INLINE_PADDING - navigatorWidth - separatorSpace - utilityWidth;
  const occupiedWidth =
    OUTER_INLINE_PADDING + navigatorWidth + separatorSpace + trackBrowserWidth + utilityWidth;

  return {
    availableWidth,
    outerInlinePadding: OUTER_INLINE_PADDING,
    separatorSpace,
    navigatorWidth,
    trackBrowserWidth,
    utilityWidth,
    fits: occupiedWidth <= availableWidth && trackBrowserWidth >= MIN_TRACK_BROWSER_WIDTH,
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function useWorkbenchLayout(viewportWidth: number, utilityOpen = false) {
  const [layout, setLayout] = useState<WorkbenchPreferencesV5>(() => loadWorkbenchPreferences());
  // Keep queued event updates composable between renders without side effects in the updater.
  const pendingLayoutRef = useRef(layout);

  const utilityWidthMax = getUtilityWidthMax(viewportWidth);
  const effectiveUtilityWidth = utilityOpen
    ? clamp(layout.utilityWidth, getUtilityWidthMin(viewportWidth), utilityWidthMax)
    : 0;
  const navigatorMax = getNavigatorWidthMax(viewportWidth, effectiveUtilityWidth);
  const navigatorCollapsed =
    layout.navigatorPreference === 'collapsed' ||
    availableNavigatorWidth(viewportWidth, effectiveUtilityWidth) < MIN_NAVIGATOR_WIDTH;
  const navigatorWidth = navigatorCollapsed
    ? COMPACT_NAVIGATOR_WIDTH
    : clamp(layout.expandedNavigatorWidth, MIN_NAVIGATOR_WIDTH, navigatorMax);
  const geometry = solveWorkbenchGeometry({
    viewportWidth,
    navigatorWidth,
    utilityWidth: effectiveUtilityWidth,
  });

  const update = (patch: WorkbenchPreferencePatch, persist = false) => {
    const next = normalizeWorkbenchPreferences({ ...pendingLayoutRef.current, ...patch });
    pendingLayoutRef.current = next;
    setLayout(() => next);
    if (persist) {
      const persisted = updateWorkbenchPreferences({
        navigatorPreference: next.navigatorPreference,
        expandedNavigatorWidth: next.expandedNavigatorWidth,
        utilityWidth: next.utilityWidth,
      });
      pendingLayoutRef.current = persisted;
      setLayout(() => persisted);
    }
  };

  return {
    ...layout,
    navigatorWidth,
    navigatorMax,
    navigatorCollapsed,
    utilityWidth: effectiveUtilityWidth,
    utilityWidthMax,
    geometry,
    setNavigatorWidth: (width: number, persist = false) =>
      update({ expandedNavigatorWidth: width, navigatorPreference: 'expanded' }, persist),
    setNavigatorPreference: (preference: NavigatorPreference, persist = false) =>
      update({ navigatorPreference: preference }, persist),
    setUtilityWidth: (width: number, persist = false) => update({ utilityWidth: width }, persist),
    resetNavigatorWidth: () =>
      update(
        { expandedNavigatorWidth: DEFAULT_NAVIGATOR_WIDTH, navigatorPreference: 'expanded' },
        true,
      ),
  };
}
