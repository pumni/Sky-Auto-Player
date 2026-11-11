import {
  COMPACT_NAVIGATOR_WIDTH,
  MAX_NAVIGATOR_WIDTH,
  MAX_UTILITY_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
} from '../../state/workbenchPreferences';

export const MIN_TRACK_BROWSER_WIDTH = 420;
export const WORKBENCH_GUTTER = 4;
export const OUTER_INLINE_PADDING = WORKBENCH_GUTTER * 2;
export const MIN_UTILITY_OVERLAY_WIDTH = 288;
export const MAX_UTILITY_OVERLAY_WIDTH = 352;
export const UTILITY_OVERLAY_WIDTH_FRACTION = 0.45;
export const AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH = 640;

export type UtilityPresentationMode = 'closed' | 'docked' | 'overlay';

export interface WorkbenchGeometryInput {
  viewportWidth: number;
  navigatorWidth: number;
  utilityWidth: number;
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

export function getAvailableNavigatorWidth(viewportWidth: number, utilityWidth: number): number {
  const separatorSpace = utilityWidth > 0 ? WORKBENCH_GUTTER * 2 : WORKBENCH_GUTTER;
  return Math.floor(
    viewportWidth - OUTER_INLINE_PADDING - separatorSpace - utilityWidth - MIN_TRACK_BROWSER_WIDTH,
  );
}

export function getNavigatorWidthMax(viewportWidth: number, utilityWidth = 0): number {
  return Math.max(
    MIN_NAVIGATOR_WIDTH,
    Math.min(MAX_NAVIGATOR_WIDTH, getAvailableNavigatorWidth(viewportWidth, utilityWidth)),
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

export function getExpandedUtilityWidthMax(viewportWidth: number): number {
  return Math.max(
    0,
    Math.min(
      MAX_UTILITY_WIDTH,
      viewportWidth -
        OUTER_INLINE_PADDING -
        WORKBENCH_GUTTER * 2 -
        MIN_NAVIGATOR_WIDTH -
        MIN_TRACK_BROWSER_WIDTH,
    ),
  );
}

export function getUtilityWidthMin(viewportWidth: number): number {
  return Math.min(MIN_UTILITY_WIDTH, getUtilityWidthMax(viewportWidth));
}

export function getUtilityPresentationMode(
  viewportWidth: number,
  utilityOpen: boolean,
): UtilityPresentationMode {
  if (!utilityOpen) return 'closed';
  return getUtilityWidthMax(viewportWidth) >= MIN_UTILITY_WIDTH ? 'docked' : 'overlay';
}

export function getUtilityOverlayWidth(viewportWidth: number): number {
  const availableWidth = Math.max(0, Math.floor(viewportWidth) - OUTER_INLINE_PADDING);
  const responsiveWidth = Math.min(
    MAX_UTILITY_OVERLAY_WIDTH,
    Math.max(MIN_UTILITY_OVERLAY_WIDTH, Math.floor(viewportWidth * UTILITY_OVERLAY_WIDTH_FRACTION)),
  );
  return Math.min(availableWidth, responsiveWidth);
}

export function solveWorkbenchGeometry({
  viewportWidth,
  navigatorWidth,
  utilityWidth,
}: WorkbenchGeometryInput): WorkbenchGeometry {
  const availableWidth = Math.max(0, Math.floor(viewportWidth));
  const gutter = WORKBENCH_GUTTER;
  const padding = OUTER_INLINE_PADDING;
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

export function clampWorkbenchValue(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}
