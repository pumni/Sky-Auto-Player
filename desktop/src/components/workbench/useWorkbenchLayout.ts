import { useEffect, useRef, useState } from 'react';

export const WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v1';
export const MIN_LIBRARY_WIDTH = 280;
export const DEFAULT_LIBRARY_WIDTH = 344;
export const MAX_LIBRARY_WIDTH = 520;
export const MIN_UTILITY_WIDTH = 300;
export const DEFAULT_UTILITY_WIDTH = 340;
export const MAX_UTILITY_WIDTH = 480;
const MIN_INSPECTOR_WIDTH = 360;
const WORKBENCH_GUTTER = 8;

export interface WorkbenchLayoutState {
  libraryWidth: number;
  utilityWidth: number;
  utilityOpen: boolean;
  version: 1;
}

export function getLibraryWidthMax(viewportWidth: number, utilityWidth = 0): number {
  const utilitySpace = utilityWidth > 0 ? utilityWidth + WORKBENCH_GUTTER : 0;
  const fixedWorkspaceSpace = MIN_INSPECTOR_WIDTH + WORKBENCH_GUTTER * 3;
  const availableForLibrary = viewportWidth - fixedWorkspaceSpace - utilitySpace;
  return Math.max(MIN_LIBRARY_WIDTH, Math.min(MAX_LIBRARY_WIDTH, availableForLibrary));
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizedLayout(
  candidate: Partial<WorkbenchLayoutState> | null | undefined,
  libraryMax = MAX_LIBRARY_WIDTH,
): WorkbenchLayoutState {
  return {
    version: 1,
    libraryWidth: clamp(
      typeof candidate?.libraryWidth === 'number' && Number.isFinite(candidate.libraryWidth)
        ? candidate.libraryWidth
        : DEFAULT_LIBRARY_WIDTH,
      MIN_LIBRARY_WIDTH,
      libraryMax,
    ),
    utilityWidth: clamp(
      typeof candidate?.utilityWidth === 'number' && Number.isFinite(candidate.utilityWidth)
        ? candidate.utilityWidth
        : DEFAULT_UTILITY_WIDTH,
      MIN_UTILITY_WIDTH,
      MAX_UTILITY_WIDTH,
    ),
    utilityOpen: candidate?.utilityOpen === true,
  };
}

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function loadWorkbenchLayout(libraryMax = MAX_LIBRARY_WIDTH): WorkbenchLayoutState {
  try {
    const raw = storage()?.getItem(WORKBENCH_STORAGE_KEY);
    if (!raw) return normalizedLayout(null, libraryMax);
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || (parsed as { version?: unknown }).version !== 1) {
      return normalizedLayout(null, libraryMax);
    }
    return normalizedLayout(parsed as Partial<WorkbenchLayoutState>, libraryMax);
  } catch {
    return normalizedLayout(null, libraryMax);
  }
}

function persistWorkbenchLayout(layout: WorkbenchLayoutState): void {
  try {
    storage()?.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Layout preferences are optional and must never prevent the app from starting.
  }
}

export function useWorkbenchLayout(viewportWidth: number, utilityVisible = false) {
  const [layout, setLayout] = useState<WorkbenchLayoutState>(() =>
    loadWorkbenchLayout(getLibraryWidthMax(viewportWidth)),
  );
  const libraryMax = getLibraryWidthMax(viewportWidth, utilityVisible ? layout.utilityWidth : 0);
  const layoutRef = useRef(layout);

  useEffect(() => {
    layoutRef.current = layout;
  }, [layout]);

  useEffect(() => {
    const next = normalizedLayout(layoutRef.current, libraryMax);
    layoutRef.current = next;
    setLayout(next);
  }, [libraryMax]);

  const update = (patch: Partial<WorkbenchLayoutState>, persist = false) => {
    const next = normalizedLayout({ ...layoutRef.current, ...patch }, libraryMax);
    layoutRef.current = next;
    setLayout(next);
    if (persist) persistWorkbenchLayout(next);
  };

  return {
    ...layout,
    libraryMax,
    setLibraryWidth: (width: number, persist = false) => update({ libraryWidth: width }, persist),
    setUtilityWidth: (width: number, persist = false) => update({ utilityWidth: width }, persist),
    setUtilityOpen: (open: boolean, persist = false) => update({ utilityOpen: open }, persist),
    resetLibraryWidth: () => update({ libraryWidth: DEFAULT_LIBRARY_WIDTH }, true),
  };
}
