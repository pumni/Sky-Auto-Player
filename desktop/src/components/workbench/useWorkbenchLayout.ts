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
import type { PaletteId } from '../../bridge/DesktopBridge';

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
export const MAX_UTILITY_OVERLAY_WIDTH = 400;
export const GRAPHITE_COMPACT_NAVIGATOR_WIDTH = 48;
export const GRAPHITE_AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH = 640;

export function getCompactNavigatorWidth(palette: PaletteId = 'classic'): number {
  return palette === 'graphite' ? GRAPHITE_COMPACT_NAVIGATOR_WIDTH : COMPACT_NAVIGATOR_WIDTH;
}

export function getWorkbenchGutter(palette: PaletteId = 'classic'): number {
  return palette === 'graphite' ? 4 : WORKBENCH_GUTTER;
}

export type UtilityPresentationMode = 'closed' | 'docked' | 'overlay';

interface WorkbenchGeometryInput {
  viewportWidth: number;
  navigatorWidth: number;
  utilityWidth: number;
  palette?: PaletteId;
}

export interface WorkbenchGeometry {
  availableWidth: number;
  outerInlinePadding: number;
  gutter: number;
  separatorSpace: number;
  navigatorWidth: number;
  trackBrowserWidth: number;
  utilityWidth: number;
  fits: boolean;
}

function outerInlinePadding(palette: PaletteId): number {
  return getWorkbenchGutter(palette) * 2;
}

function availableNavigatorWidth(
  viewportWidth: number,
  utilityWidth: number,
  palette: PaletteId,
): number {
  const gutter = getWorkbenchGutter(palette);
  const separatorSpace = utilityWidth > 0 ? gutter * 2 : gutter;
  return Math.floor(
    viewportWidth -
      outerInlinePadding(palette) -
      separatorSpace -
      utilityWidth -
      MIN_TRACK_BROWSER_WIDTH,
  );
}

export function getNavigatorWidthMax(
  viewportWidth: number,
  utilityWidth = 0,
  palette: PaletteId = 'classic',
): number {
  return Math.max(
    MIN_NAVIGATOR_WIDTH,
    Math.min(MAX_NAVIGATOR_WIDTH, availableNavigatorWidth(viewportWidth, utilityWidth, palette)),
  );
}

export function getUtilityWidthMax(viewportWidth: number, palette: PaletteId = 'classic'): number {
  const gutter = getWorkbenchGutter(palette);
  const available =
    viewportWidth -
    outerInlinePadding(palette) -
    gutter * 2 -
    getCompactNavigatorWidth(palette) -
    MIN_TRACK_BROWSER_WIDTH;
  return Math.min(MAX_UTILITY_WIDTH, Math.max(0, Math.floor(available)));
}

export function getUtilityWidthMin(viewportWidth: number, palette: PaletteId = 'classic'): number {
  return Math.min(MIN_UTILITY_WIDTH, getUtilityWidthMax(viewportWidth, palette));
}

export function getUtilityPresentationMode(
  viewportWidth: number,
  utilityOpen: boolean,
  palette: PaletteId = 'classic',
): UtilityPresentationMode {
  if (!utilityOpen) return 'closed';
  return getUtilityWidthMax(viewportWidth, palette) >= MIN_UTILITY_WIDTH ? 'docked' : 'overlay';
}

export function getUtilityOverlayWidth(
  viewportWidth: number,
  palette: PaletteId = 'classic',
): number {
  const availableWidth = Math.max(0, Math.floor(viewportWidth) - outerInlinePadding(palette));
  const responsiveWidth = Math.max(MIN_UTILITY_WIDTH, Math.floor(viewportWidth / 2));
  return Math.min(MAX_UTILITY_OVERLAY_WIDTH, responsiveWidth, availableWidth);
}

export function solveWorkbenchGeometry({
  viewportWidth,
  navigatorWidth,
  utilityWidth,
  palette = 'classic',
}: WorkbenchGeometryInput): WorkbenchGeometry {
  const availableWidth = Math.max(0, Math.floor(viewportWidth));
  const gutter = getWorkbenchGutter(palette);
  const padding = outerInlinePadding(palette);
  const separatorSpace = utilityWidth > 0 ? gutter * 2 : gutter;
  const trackBrowserWidth =
    availableWidth - padding - navigatorWidth - separatorSpace - utilityWidth;
  const occupiedWidth =
    padding + navigatorWidth + separatorSpace + trackBrowserWidth + utilityWidth;

  return {
    availableWidth,
    outerInlinePadding: padding,
    gutter,
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

export function useWorkbenchLayout(
  viewportWidth: number,
  utilityDocked = false,
  palette: PaletteId = 'classic',
  viewportRevision = 0,
) {
  const [layout, setLayout] = useState<WorkbenchPreferencesV5>(() => loadWorkbenchPreferences());
  const [narrowExpansionOverrideRevision, setNarrowExpansionOverrideRevision] = useState<
    number | null
  >(null);
  // Keep queued event updates composable between renders without side effects in the updater.
  const pendingLayoutRef = useRef(layout);

  const utilityWidthMax = getUtilityWidthMax(viewportWidth, palette);
  const effectiveUtilityWidth = utilityDocked
    ? clamp(layout.utilityWidth, getUtilityWidthMin(viewportWidth, palette), utilityWidthMax)
    : 0;
  const navigatorMax = getNavigatorWidthMax(viewportWidth, effectiveUtilityWidth, palette);
  const narrowGraphiteWindow =
    palette === 'graphite' && viewportWidth <= GRAPHITE_AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH;
  const narrowExpansionOverride =
    narrowGraphiteWindow && narrowExpansionOverrideRevision === viewportRevision;
  const navigatorCollapsed =
    layout.navigatorPreference === 'collapsed' ||
    (narrowGraphiteWindow && !narrowExpansionOverride) ||
    availableNavigatorWidth(viewportWidth, effectiveUtilityWidth, palette) < MIN_NAVIGATOR_WIDTH;
  const navigatorWidth = navigatorCollapsed
    ? getCompactNavigatorWidth(palette)
    : clamp(layout.expandedNavigatorWidth, MIN_NAVIGATOR_WIDTH, navigatorMax);
  const geometry = solveWorkbenchGeometry({
    viewportWidth,
    navigatorWidth,
    utilityWidth: effectiveUtilityWidth,
    palette,
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
    setNavigatorPreference: (preference: NavigatorPreference, persist = false) => {
      setNarrowExpansionOverrideRevision(
        narrowGraphiteWindow && preference === 'expanded' ? viewportRevision : null,
      );
      update({ navigatorPreference: preference }, persist);
    },
    setUtilityWidth: (width: number, persist = false) => update({ utilityWidth: width }, persist),
    resetNavigatorWidth: () =>
      update(
        { expandedNavigatorWidth: DEFAULT_NAVIGATOR_WIDTH, navigatorPreference: 'expanded' },
        true,
      ),
  };
}
