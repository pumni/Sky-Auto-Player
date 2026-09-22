export const WORKBENCH_STORAGE_KEY = 'sky.ui.workbench.v5';
export const LEGACY_WORKBENCH_V4_STORAGE_KEY = 'sky.ui.workbench.v4';
export const LEGACY_WORKBENCH_V3_STORAGE_KEY = 'sky.ui.workbench.v3';
export const LEGACY_WORKBENCH_V2_STORAGE_KEY = 'sky.ui.workbench.v2';
export const LEGACY_WORKBENCH_V1_STORAGE_KEY = 'sky.ui.workbench.v1';

// Keep the existing v2 export available to minimize call-site churn.
export const LEGACY_WORKBENCH_STORAGE_KEY = LEGACY_WORKBENCH_V2_STORAGE_KEY;

export const MIN_NAVIGATOR_WIDTH = 200;
export const COMPACT_NAVIGATOR_WIDTH = 56;
export const DEFAULT_NAVIGATOR_WIDTH = 240;
export const MAX_NAVIGATOR_WIDTH = 340;
export const MIN_UTILITY_WIDTH = 280;
export const DEFAULT_UTILITY_WIDTH = 320;
export const MAX_UTILITY_WIDTH = 440;

export type NavigatorPreference = 'expanded' | 'collapsed';

export interface WorkbenchPreferencesV5 {
  version: 5;
  navigatorPreference: NavigatorPreference;
  expandedNavigatorWidth: number;
  utilityWidth: number;
  utilityOpen: boolean;
}

export type WorkbenchPreferencePatch = Partial<Omit<WorkbenchPreferencesV5, 'version'>>;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function normalizeWorkbenchPreferences(
  candidate: Partial<WorkbenchPreferencesV5> | null | undefined,
): WorkbenchPreferencesV5 {
  return {
    version: 5,
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
    utilityOpen: typeof candidate?.utilityOpen === 'boolean' ? candidate.utilityOpen : false,
  };
}

function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
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

export function loadWorkbenchPreferences(): WorkbenchPreferencesV5 {
  const current = readObject(WORKBENCH_STORAGE_KEY);
  if (current?.version === 5) {
    return normalizeWorkbenchPreferences(current as Partial<WorkbenchPreferencesV5>);
  }

  const legacyV4 = readObject(LEGACY_WORKBENCH_V4_STORAGE_KEY);
  if (legacyV4?.version === 4) {
    return normalizeWorkbenchPreferences({
      navigatorPreference: legacyV4.navigatorPreference as NavigatorPreference,
      expandedNavigatorWidth: legacyV4.expandedNavigatorWidth as number,
      utilityWidth: legacyV4.utilityWidth as number,
    });
  }

  const legacyV3 = readObject(LEGACY_WORKBENCH_V3_STORAGE_KEY);
  if (legacyV3?.version === 3) {
    return normalizeWorkbenchPreferences({
      navigatorPreference: legacyV3.navigatorPreference as NavigatorPreference,
      expandedNavigatorWidth: legacyV3.expandedNavigatorWidth as number,
      utilityWidth: legacyV3.utilityWidth as number,
    });
  }

  const legacyV2 = readObject(LEGACY_WORKBENCH_V2_STORAGE_KEY);
  if (legacyV2?.version === 2) {
    return normalizeWorkbenchPreferences({
      expandedNavigatorWidth: legacyV2.navigatorWidth as number,
      utilityWidth: legacyV2.utilityWidth as number,
    });
  }

  const legacyV1 = readObject(LEGACY_WORKBENCH_V1_STORAGE_KEY);
  if (legacyV1?.version === 1) {
    return normalizeWorkbenchPreferences({
      expandedNavigatorWidth: legacyV1.libraryWidth as number,
      utilityWidth: legacyV1.utilityWidth as number,
    });
  }

  return normalizeWorkbenchPreferences(null);
}

export function updateWorkbenchPreferences(
  patch: WorkbenchPreferencePatch,
): WorkbenchPreferencesV5 {
  const next = normalizeWorkbenchPreferences({ ...loadWorkbenchPreferences(), ...patch });
  try {
    storage()?.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Workbench preferences are optional and must never prevent the app from starting.
  }
  return next;
}
