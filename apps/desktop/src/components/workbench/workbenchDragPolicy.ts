import { getExpandedUtilityWidthMax, getUtilityWidthMax } from './workbenchGeometry';
import type { NavigatorPreference } from '../../state/workbenchPreferences';

export interface UtilityDragTopology {
  collapsed: boolean;
  dragging: boolean;
  width: number;
  revision: number;
}

export interface UtilityDragTopologyResolution {
  expired: boolean;
  collapsed: boolean | null;
}

export function createUtilityDragTopology(
  collapsed: boolean,
  dragging: boolean,
  width: number,
  revision: number,
): UtilityDragTopology {
  return { collapsed, dragging, width, revision };
}

export function resolveUtilityDragTopology(
  topology: UtilityDragTopology | null,
  utilityDocked: boolean,
  viewportWidth: number,
  viewportRevision: number,
): UtilityDragTopologyResolution {
  const expired =
    topology !== null &&
    ((topology.dragging && !utilityDocked) ||
      (!topology.dragging &&
        (topology.width !== viewportWidth || topology.revision !== viewportRevision)));

  return {
    expired,
    collapsed: expired ? null : (topology?.collapsed ?? null),
  };
}

export function updateUtilityDragTopologyForNavigatorPreference(
  topology: UtilityDragTopology | null,
  preference: NavigatorPreference,
): UtilityDragTopology | null {
  return topology?.dragging ? { ...topology, collapsed: preference === 'collapsed' } : null;
}

export function getUtilityWidthMaxForDrag(
  viewportWidth: number,
  collapsed: boolean | null,
): number {
  return collapsed === false
    ? getExpandedUtilityWidthMax(viewportWidth)
    : getUtilityWidthMax(viewportWidth);
}

export const NAVIGATOR_COLLAPSE_THRESHOLD = 116;
export const NAVIGATOR_EXPAND_THRESHOLD = 136;
export const UTILITY_COLLAPSE_THRESHOLD = 156;
export const UTILITY_EXPAND_THRESHOLD = 180;

export type DragSnapHint = 'collapse' | 'expand' | null;

export interface PaneDragInput {
  currentCollapsed: boolean;
  rawWidth: number;
  compactWidth: number;
  minExpandedWidth: number;
  maxExpandedWidth: number;
  collapseThreshold: number;
  expandThreshold: number;
  canExpand?: boolean;
}

export interface PaneDragResolution {
  collapsed: boolean;
  width: number;
  snapHint: DragSnapHint;
}

export function resolvePaneDrag({
  currentCollapsed,
  rawWidth,
  compactWidth,
  minExpandedWidth,
  maxExpandedWidth,
  collapseThreshold,
  expandThreshold,
  canExpand = true,
}: PaneDragInput): PaneDragResolution {
  const effectiveMax = Math.max(minExpandedWidth, maxExpandedWidth);
  const midpoint = Math.round((compactWidth + minExpandedWidth) / 2);
  const effectiveCollapse = Math.min(collapseThreshold, Math.max(compactWidth + 1, midpoint - 10));
  const effectiveExpand = Math.max(
    effectiveCollapse + 12,
    Math.min(expandThreshold, Math.max(effectiveCollapse + 12, minExpandedWidth - 8)),
  );

  if (currentCollapsed) {
    if (!canExpand) {
      return { collapsed: true, width: compactWidth, snapHint: null };
    }
    if (rawWidth > effectiveExpand) {
      return {
        collapsed: false,
        width: Math.min(effectiveMax, Math.max(minExpandedWidth, Math.round(rawWidth))),
        snapHint: null,
      };
    }
    return {
      collapsed: true,
      width: compactWidth,
      snapHint: rawWidth > compactWidth + 12 ? 'expand' : null,
    };
  }

  if (rawWidth < effectiveCollapse) {
    return { collapsed: true, width: compactWidth, snapHint: null };
  }

  const clamped = Math.min(effectiveMax, Math.max(minExpandedWidth, Math.round(rawWidth)));
  return {
    collapsed: false,
    width: clamped,
    snapHint: rawWidth < minExpandedWidth - 12 ? 'collapse' : null,
  };
}
