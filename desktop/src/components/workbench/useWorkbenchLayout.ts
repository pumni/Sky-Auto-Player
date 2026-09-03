import { useEffect, useRef, useState } from 'react';

export const WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v2';
export const LEGACY_WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v1';
export const MIN_NAVIGATOR_WIDTH = 220;
export const DEFAULT_NAVIGATOR_WIDTH = 260;
export const MAX_NAVIGATOR_WIDTH = 360;
export const MIN_UTILITY_WIDTH = 320;
export const DEFAULT_UTILITY_WIDTH = 360;
export const MAX_UTILITY_WIDTH = 480;
export const MIN_TRACK_BROWSER_WIDTH = 480;
export const WORKBENCH_GUTTER = 8;

export interface WorkbenchLayoutStateV2 {
  version: 2;
  navigatorWidth: number;
  utilityWidth: number;
}

export function getNavigatorWidthMax(viewportWidth: number, utilityWidth = 0): number {
  const utilitySpace = utilityWidth > 0 ? utilityWidth + WORKBENCH_GUTTER : 0;
  const workbenchPadding = WORKBENCH_GUTTER * 2;
  const separators = utilityWidth > 0 ? WORKBENCH_GUTTER * 2 : WORKBENCH_GUTTER;
  const availableForNavigator =
    viewportWidth - workbenchPadding - separators - utilitySpace - MIN_TRACK_BROWSER_WIDTH;
  return Math.max(
    MIN_NAVIGATOR_WIDTH,
    Math.min(MAX_NAVIGATOR_WIDTH, Math.floor(availableForNavigator)),
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizedLayout(
  candidate: Partial<WorkbenchLayoutStateV2> | null | undefined,
  navigatorMax = MAX_NAVIGATOR_WIDTH,
): WorkbenchLayoutStateV2 {
  return {
    version: 2,
    navigatorWidth: clamp(
      typeof candidate?.navigatorWidth === 'number' && Number.isFinite(candidate.navigatorWidth)
        ? candidate.navigatorWidth
        : DEFAULT_NAVIGATOR_WIDTH,
      MIN_NAVIGATOR_WIDTH,
      navigatorMax,
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

export function loadWorkbenchLayout(navigatorMax = MAX_NAVIGATOR_WIDTH): WorkbenchLayoutStateV2 {
  const current = readObject(WORKBENCH_STORAGE_KEY);
  if (current?.version === 2) {
    return normalizedLayout(current as Partial<WorkbenchLayoutStateV2>, navigatorMax);
  }

  const legacy = readObject(LEGACY_WORKBENCH_STORAGE_KEY);
  if (legacy?.version === 1) {
    return normalizedLayout(
      {
        navigatorWidth: legacy.libraryWidth as number,
        utilityWidth: legacy.utilityWidth as number,
      },
      navigatorMax,
    );
  }

  return normalizedLayout(null, navigatorMax);
}

function persistWorkbenchLayout(layout: WorkbenchLayoutStateV2): void {
  try {
    storage()?.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Layout preferences are optional and must never prevent the app from starting.
  }
}

export function useWorkbenchLayout(viewportWidth: number, utilityVisible = false) {
  const [layout, setLayout] = useState<WorkbenchLayoutStateV2>(() =>
    loadWorkbenchLayout(getNavigatorWidthMax(viewportWidth)),
  );
  const navigatorMax = getNavigatorWidthMax(
    viewportWidth,
    utilityVisible ? layout.utilityWidth : 0,
  );
  const layoutRef = useRef(layout);

  useEffect(() => {
    layoutRef.current = layout;
  }, [layout]);

  useEffect(() => {
    const next = normalizedLayout(layoutRef.current, navigatorMax);
    layoutRef.current = next;
    setLayout(next);
  }, [navigatorMax]);

  const update = (patch: Partial<WorkbenchLayoutStateV2>, persist = false) => {
    const next = normalizedLayout({ ...layoutRef.current, ...patch }, navigatorMax);
    layoutRef.current = next;
    setLayout(next);
    if (persist) persistWorkbenchLayout(next);
  };

  return {
    ...layout,
    navigatorMax,
    setNavigatorWidth: (width: number, persist = false) =>
      update({ navigatorWidth: width }, persist),
    setUtilityWidth: (width: number, persist = false) => update({ utilityWidth: width }, persist),
    resetNavigatorWidth: () => update({ navigatorWidth: DEFAULT_NAVIGATOR_WIDTH }, true),
  };
}
