import { describe, expect, it } from 'vitest';
import {
  createUtilityDragTopology,
  getUtilityWidthMaxForDrag,
  NAVIGATOR_COLLAPSE_THRESHOLD,
  NAVIGATOR_EXPAND_THRESHOLD,
  resolvePaneDrag,
  resolveUtilityDragTopology,
  updateUtilityDragTopologyForNavigatorPreference,
  UTILITY_COLLAPSE_THRESHOLD,
  UTILITY_EXPAND_THRESHOLD,
} from './workbenchDragPolicy';

describe('workbench drag policy', () => {
  it('keeps a live docked drag topology across resize rebases and expires it when docked bounds change after release', () => {
    const dragging = createUtilityDragTopology(false, true, 1024, 4);
    expect(resolveUtilityDragTopology(dragging, true, 900, 5)).toEqual({
      expired: false,
      collapsed: false,
    });

    const released = createUtilityDragTopology(false, false, 900, 5);
    expect(resolveUtilityDragTopology(released, true, 900, 5)).toEqual({
      expired: false,
      collapsed: false,
    });
    expect(resolveUtilityDragTopology(released, true, 901, 6)).toEqual({
      expired: true,
      collapsed: null,
    });
  });

  it('expires an unfinished drag when the docked Inspector becomes an overlay', () => {
    expect(
      resolveUtilityDragTopology(createUtilityDragTopology(false, true, 800, 1), false, 640, 2),
    ).toEqual({ expired: true, collapsed: null });
  });

  it('updates only a live drag topology for explicit navigator preferences', () => {
    const dragging = createUtilityDragTopology(false, true, 1024, 2);
    expect(updateUtilityDragTopologyForNavigatorPreference(dragging, 'collapsed')).toEqual({
      ...dragging,
      collapsed: true,
    });
    expect(
      updateUtilityDragTopologyForNavigatorPreference(
        createUtilityDragTopology(false, false, 1024, 2),
        'collapsed',
      ),
    ).toBeNull();
  });

  it('uses expanded width limits only while the drag explicitly keeps the navigator expanded', () => {
    expect(getUtilityWidthMaxForDrag(1024, false)).toBe(388);
    expect(getUtilityWidthMaxForDrag(1024, true)).toBe(440);
    expect(getUtilityWidthMaxForDrag(1024, null)).toBe(440);
  });

  it('resolves magnetic collapse, resistance dead-zone, and hysteresis expansion for Library Navigator', () => {
    const base = {
      compactWidth: 48,
      minExpandedWidth: 200,
      maxExpandedWidth: 340,
      collapseThreshold: NAVIGATOR_COLLAPSE_THRESHOLD,
      expandThreshold: NAVIGATOR_EXPAND_THRESHOLD,
    };

    // Normal expanded drag clamps cleanly inside [200, 340]
    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 265 })).toEqual({
      collapsed: false,
      width: 265,
      snapHint: null,
    });
    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 420 })).toEqual({
      collapsed: false,
      width: 340,
      snapHint: null,
    });

    // Resistance dead-zone holds at minExpandedWidth and raises collapse hint
    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 160 })).toEqual({
      collapsed: false,
      width: 200,
      snapHint: 'collapse',
    });

    // Crossing below collapseThreshold snaps to 48px
    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 110 })).toEqual({
      collapsed: true,
      width: 48,
      snapHint: null,
    });

    // Hysteresis: once collapsed, staying between collapseThreshold and expandThreshold stays collapsed
    expect(resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 128 })).toEqual({
      collapsed: true,
      width: 48,
      snapHint: 'expand',
    });

    // Crossing past expandThreshold snaps open and follows cursor immediately
    expect(resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 145 })).toEqual({
      collapsed: false,
      width: 200,
      snapHint: null,
    });
    expect(resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 250 })).toEqual({
      collapsed: false,
      width: 250,
      snapHint: null,
    });

    // When expansion is disallowed (narrow viewport), stays locked at compact width
    expect(
      resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 260, canExpand: false }),
    ).toEqual({
      collapsed: true,
      width: 48,
      snapHint: null,
    });
  });

  it('resolves magnetic collapse and hysteresis expansion for docked Inspector', () => {
    const base = {
      compactWidth: 48,
      minExpandedWidth: 280,
      maxExpandedWidth: 440,
      collapseThreshold: UTILITY_COLLAPSE_THRESHOLD,
      expandThreshold: UTILITY_EXPAND_THRESHOLD,
    };

    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 220 })).toEqual({
      collapsed: false,
      width: 280,
      snapHint: 'collapse',
    });
    expect(resolvePaneDrag({ ...base, currentCollapsed: false, rawWidth: 150 })).toEqual({
      collapsed: true,
      width: 48,
      snapHint: null,
    });
    expect(resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 170 })).toEqual({
      collapsed: true,
      width: 48,
      snapHint: 'expand',
    });
    expect(resolvePaneDrag({ ...base, currentCollapsed: true, rawWidth: 190 })).toEqual({
      collapsed: false,
      width: 280,
      snapHint: null,
    });
  });
});
