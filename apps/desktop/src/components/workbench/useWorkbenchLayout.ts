import { useState } from 'react';
import {
  AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH,
  getAvailableNavigatorWidth,
  getExpandedUtilityWidthMax,
  getNavigatorWidthMax,
  solveWorkbenchGeometry,
  clampWorkbenchValue,
} from './workbenchGeometry';
import {
  COMPACT_NAVIGATOR_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
} from '../../state/workbenchPreferences';
import {
  createUtilityDragTopology,
  getUtilityWidthMaxForDrag,
  resolveUtilityDragTopology,
  updateUtilityDragTopologyForNavigatorPreference,
} from './workbenchDragPolicy';
import type { UtilityDragTopology } from './workbenchDragPolicy';
import { useWorkbenchPreferences } from './useWorkbenchPreferences';
import type { NavigatorPreference, WorkbenchPreferencesV5 } from '../../state/workbenchPreferences';

export {
  AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH,
  OUTER_INLINE_PADDING,
  MIN_TRACK_BROWSER_WIDTH,
  MIN_UTILITY_OVERLAY_WIDTH,
  MAX_UTILITY_OVERLAY_WIDTH,
  UTILITY_OVERLAY_WIDTH_FRACTION,
  WORKBENCH_GUTTER,
  getNavigatorWidthMax,
  getUtilityOverlayWidth,
  getUtilityPresentationMode,
  getUtilityWidthMax,
  getUtilityWidthMin,
  solveWorkbenchGeometry,
} from './workbenchGeometry';
export {
  COMPACT_NAVIGATOR_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  DEFAULT_UTILITY_WIDTH,
  MAX_NAVIGATOR_WIDTH,
  MAX_UTILITY_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
  LEGACY_WORKBENCH_STORAGE_KEY,
  LEGACY_WORKBENCH_V1_STORAGE_KEY,
  LEGACY_WORKBENCH_V2_STORAGE_KEY,
  LEGACY_WORKBENCH_V3_STORAGE_KEY,
  LEGACY_WORKBENCH_V4_STORAGE_KEY,
  WORKBENCH_STORAGE_KEY,
  loadWorkbenchPreferences as loadWorkbenchLayout,
  normalizeWorkbenchPreferences,
  updateWorkbenchPreferences,
} from '../../state/workbenchPreferences';
export type { UtilityPresentationMode, WorkbenchGeometry } from './workbenchGeometry';
export type { NavigatorPreference, WorkbenchPreferencesV5 };
export type WorkbenchLayoutStateV4 = WorkbenchPreferencesV5;

export function useWorkbenchLayout(
  viewportWidth: number,
  utilityDocked = false,
  viewportRevision = 0,
) {
  const { layout, update } = useWorkbenchPreferences();
  const [narrowExpansionOverrideRevision, setNarrowExpansionOverrideRevision] = useState<
    number | null
  >(null);
  const [utilityTopology, setUtilityTopology] = useState<UtilityDragTopology | null>(null);
  const topology = resolveUtilityDragTopology(
    utilityTopology,
    utilityDocked,
    viewportWidth,
    viewportRevision,
  );
  if (topology.expired) setUtilityTopology(null);
  const utilityDragCollapsed = topology.collapsed;

  const expandedUtilityMax = getExpandedUtilityWidthMax(viewportWidth);
  const utilityWidthMax = getUtilityWidthMaxForDrag(viewportWidth, utilityDragCollapsed);
  const effectiveUtilityWidth = utilityDocked
    ? clampWorkbenchValue(
        layout.utilityWidth,
        Math.min(MIN_UTILITY_WIDTH, utilityWidthMax),
        utilityWidthMax,
      )
    : 0;
  const navigatorMax = getNavigatorWidthMax(viewportWidth, effectiveUtilityWidth);
  const narrowWindow = viewportWidth <= AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH;
  const narrowExpansionOverride =
    narrowWindow && narrowExpansionOverrideRevision === viewportRevision;
  const responsiveCollapsed =
    layout.navigatorPreference === 'collapsed' ||
    (narrowWindow && !narrowExpansionOverride) ||
    getAvailableNavigatorWidth(viewportWidth, effectiveUtilityWidth) < MIN_NAVIGATOR_WIDTH;
  const navigatorCollapsed = utilityDragCollapsed ?? responsiveCollapsed;
  const navigatorWidth = navigatorCollapsed
    ? COMPACT_NAVIGATOR_WIDTH
    : clampWorkbenchValue(layout.expandedNavigatorWidth, MIN_NAVIGATOR_WIDTH, navigatorMax);
  const geometry = solveWorkbenchGeometry({
    viewportWidth,
    navigatorWidth,
    utilityWidth: effectiveUtilityWidth,
  });

  return {
    ...layout,
    navigatorWidth,
    navigatorMax,
    navigatorCollapsed,
    utilityWidth: effectiveUtilityWidth,
    utilityWidthMax: navigatorCollapsed ? utilityWidthMax : expandedUtilityMax,
    beginUtilityDrag: () =>
      setUtilityTopology(
        createUtilityDragTopology(navigatorCollapsed, true, viewportWidth, viewportRevision),
      ),
    // v5 may save >=280 while the current viewport only permits 264. Retain the
    // effective topology until a later viewport or explicit navigator action.
    endUtilityDrag: () =>
      setUtilityTopology(
        createUtilityDragTopology(navigatorCollapsed, false, viewportWidth, viewportRevision),
      ),
    geometry,
    setNavigatorWidth: (width: number, persist = false) =>
      update({ expandedNavigatorWidth: width, navigatorPreference: 'expanded' }, persist),
    setNavigatorPreference: (preference: NavigatorPreference, persist = false) => {
      setUtilityTopology((current) =>
        updateUtilityDragTopologyForNavigatorPreference(current, preference),
      );
      setNarrowExpansionOverrideRevision(
        narrowWindow && preference === 'expanded' ? viewportRevision : null,
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
