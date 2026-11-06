import { describe, expect, it } from 'vitest';
import {
  createUtilityDragTopology,
  getUtilityWidthMaxForDrag,
  resolveUtilityDragTopology,
  updateUtilityDragTopologyForNavigatorPreference,
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
});
