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
