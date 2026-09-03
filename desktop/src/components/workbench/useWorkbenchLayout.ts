import { useEffect, useRef, useState } from 'react';

export const WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v4';
export const LEGACY_WORKBENCH_V3_STORAGE_KEY = 'sky.ui.workbench.v3';
export const LEGACY_WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v2';
export const LEGACY_WORKBENCH_V1_STORAGE_KEY = 'sky.ui.workbench.v1';
export const MIN_NAVIGATOR_WIDTH = 200;
export const COMPACT_NAVIGATOR_WIDTH = 56;
export const DEFAULT_NAVIGATOR_WIDTH = 240;
export const MAX_NAVIGATOR_WIDTH = 340;
export const MIN_UTILITY_WIDTH = 280;
export const DEFAULT_UTILITY_WIDTH = 320;
export const MAX_UTILITY_WIDTH = 440;
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

export type NavigatorPreference = 'expanded' | 'collapsed';

export interface WorkbenchLayoutStateV4 {
  version: 4;
  navigatorPreference: NavigatorPreference;
  expandedNavigatorWidth: number;
  utilityWidth: number;
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

function normalizedLayout(
  candidate: Partial<WorkbenchLayoutStateV4> | null | undefined,
): WorkbenchLayoutStateV4 {
  return {
    version: 4,
    navigatorPreference: candidate?.navigatorPreference === 'collapsed' ? 'collapsed' : 'expanded',
    expandedNavigatorWidth: clamp(
      typeof candidate?.expandedNavigatorWidth === 'number' &&
        Number.isFinite(candidate.expandedNavigatorWidth)
        ? candidate.expandedNavigatorWidth
        : DEFAULT_NAVIGATOR_WIDTH,
      MIN_NAVIGATOR_WIDTH,
      MAX_NAVIGATOR_WIDTH,
    ),
    utilityWidth: clamp(
      typeof candidate?.utilityWidth === 'number' && Number.isFinite(candidate.utilityWidth)
        ? candidate.utilityWidth
        : DEFAULT_UTILITY_WIDTH,
      MIN_UTILITY_WIDTH,
      MAX_UTILITY_WIDTH,
    ),
  };
}

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function readObject(key: string): Record<string, unknown> | null {
  try {
    const raw = storage()?.getItem(key);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

export function loadWorkbenchLayout(): WorkbenchLayoutStateV4 {
  const current = readObject(WORKBENCH_STORAGE_KEY);
  if (current?.version === 4) return normalizedLayout(current as Partial<WorkbenchLayoutStateV4>);

  const legacyV3 = readObject(LEGACY_WORKBENCH_V3_STORAGE_KEY);
  if (legacyV3?.version === 3) {
    return normalizedLayout({
      navigatorPreference: legacyV3.navigatorPreference as NavigatorPreference,
      expandedNavigatorWidth: legacyV3.expandedNavigatorWidth as number,
      utilityWidth: legacyV3.utilityWidth as number,
    });
  }

  const legacyV2 = readObject(LEGACY_WORKBENCH_STORAGE_KEY);
  if (legacyV2?.version === 2) {
    return normalizedLayout({
      expandedNavigatorWidth: legacyV2.navigatorWidth as number,
      utilityWidth: legacyV2.utilityWidth as number,
    });
  }

  const legacyV1 = readObject(LEGACY_WORKBENCH_V1_STORAGE_KEY);
  if (legacyV1?.version === 1) {
    return normalizedLayout({
      expandedNavigatorWidth: legacyV1.libraryWidth as number,
      utilityWidth: legacyV1.utilityWidth as number,
    });
  }

  return normalizedLayout(null);
}

function persistWorkbenchLayout(layout: WorkbenchLayoutStateV4): void {
  try {
    storage()?.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Layout preferences are optional and must never prevent the app from starting.
  }
}

export function useWorkbenchLayout(viewportWidth: number, utilityOpen = false) {
  const [layout, setLayout] = useState<WorkbenchLayoutStateV4>(() => loadWorkbenchLayout());
  const layoutRef = useRef(layout);

  useEffect(() => {
    layoutRef.current = layout;
  }, [layout]);

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

  const update = (patch: Partial<WorkbenchLayoutStateV4>, persist = false) => {
    const next = normalizedLayout({ ...layoutRef.current, ...patch });
    layoutRef.current = next;
    setLayout(next);
    if (persist) persistWorkbenchLayout(next);
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
