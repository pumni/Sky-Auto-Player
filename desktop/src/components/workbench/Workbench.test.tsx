import { StrictMode } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ResizableSeparator } from './ResizableSeparator';
import {
  GRAPHITE_AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  COMPACT_NAVIGATOR_WIDTH,
  getWorkbenchGutter,
  getCompactNavigatorWidth,
  getNavigatorWidthMax,
  LEGACY_WORKBENCH_STORAGE_KEY,
  LEGACY_WORKBENCH_V4_STORAGE_KEY,
  LEGACY_WORKBENCH_V3_STORAGE_KEY,
  LEGACY_WORKBENCH_V1_STORAGE_KEY,
  loadWorkbenchLayout,
  MIN_NAVIGATOR_WIDTH,
  getUtilityWidthMax,
  getUtilityOverlayWidth,
  getUtilityPresentationMode,
  solveWorkbenchGeometry,
  updateWorkbenchPreferences,
  WORKBENCH_STORAGE_KEY,
  useWorkbenchLayout,
} from './useWorkbenchLayout';

function LayoutUpdateHarness() {
  const layout = useWorkbenchLayout(1200, true);
  return (
    <>
      <output data-testid="layout-values">
        {layout.expandedNavigatorWidth}:{layout.utilityWidth}
      </output>
      <button
        type="button"
        onClick={() => {
          layout.setNavigatorWidth(300);
          layout.setUtilityWidth(400);
        }}
      >
        Update layout
      </button>
      <button type="button" onClick={() => layout.setUtilityWidth(410, true)}>
        Commit utility width
      </button>
    </>
  );
}

function ResponsiveNavigatorHarness({
  viewportWidth,
  viewportRevision = 0,
}: {
  viewportWidth: number;
  viewportRevision?: number;
}) {
  const layout = useWorkbenchLayout(viewportWidth, false, 'graphite', viewportRevision);
  return (
    <>
      <output data-testid="responsive-navigator">
        {`${layout.navigatorCollapsed}:${layout.navigatorWidth}:${layout.navigatorPreference}:${layout.expandedNavigatorWidth}`}
      </output>
      <button type="button" onClick={() => layout.setNavigatorPreference('expanded')}>
        Expand navigator
      </button>
    </>
  );
}

describe('ResizableSeparator', () => {
  afterEach(() => window.localStorage.clear());

  it('supports keyboard resize, clamping, and reset', () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResizableSeparator
        label="Resize library navigator"
        value={260}
        min={200}
        max={340}
        defaultValue={DEFAULT_NAVIGATOR_WIDTH}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole('separator', { name: 'Resize library navigator' });

    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(268);
    expect(onCommit).toHaveBeenLastCalledWith(268);

    rerender(
      <ResizableSeparator
        label="Resize library navigator"
        value={268}
        min={200}
        max={340}
        defaultValue={DEFAULT_NAVIGATOR_WIDTH}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'ArrowLeft',
      shiftKey: true,
    });
    expect(onChange).toHaveBeenLastCalledWith(236);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'Home',
    });
    expect(onChange).toHaveBeenLastCalledWith(200);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'End',
    });
    expect(onChange).toHaveBeenLastCalledWith(340);
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Resize library navigator' }), {
      key: 'Enter',
    });
    expect(onChange).toHaveBeenLastCalledWith(DEFAULT_NAVIGATOR_WIDTH);
  });

  it('reverses pointer semantics for a pane owned after the separator', () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const { rerender } = render(
      <ResizableSeparator
        label="Resize utility pane"
        value={360}
        min={280}
        max={440}
        defaultValue={320}
        direction={-1}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole('separator', { name: 'Resize utility pane' });

    fireEvent.keyDown(separator, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(352);
    rerender(
      <ResizableSeparator
        label="Resize utility pane"
        value={352}
        min={280}
        max={440}
        defaultValue={320}
        direction={-1}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    fireEvent.keyDown(separator, { key: 'ArrowLeft', shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(384);
    expect(onCommit).toHaveBeenLastCalledWith(384);
  });
});

describe('workbench layout persistence', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it('loads valid v5 values and clamps widths to current bounds', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'collapsed',
        expandedNavigatorWidth: 640,
        utilityWidth: 490,
        utilityOpen: true,
      }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 5,
      navigatorPreference: 'collapsed',
      expandedNavigatorWidth: 340,
      utilityWidth: 440,
      utilityOpen: true,
    });
  });

  it('soft-migrates valid v4 geometry and defaults utility open to false', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_V4_STORAGE_KEY,
      JSON.stringify({
        version: 4,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 640,
        utilityWidth: 340,
      }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 340,
      utilityWidth: 340,
      utilityOpen: false,
    });
  });

  it('soft-migrates valid v3 geometry and defaults utility open to false', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_V3_STORAGE_KEY,
      JSON.stringify({
        version: 3,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 360,
        utilityWidth: 340,
      }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 340,
      utilityWidth: 340,
      utilityOpen: false,
    });
  });

  it('soft-migrates valid v2 geometry and defaults utility open to false', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_STORAGE_KEY,
      JSON.stringify({ version: 2, navigatorWidth: 280, utilityWidth: 490 }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 280,
      utilityWidth: 440,
      utilityOpen: false,
    });
  });

  it('soft-migrates v1 geometry and ignores historical utility open state', () => {
    window.localStorage.setItem(
      LEGACY_WORKBENCH_V1_STORAGE_KEY,
      JSON.stringify({ version: 1, libraryWidth: 280, utilityWidth: 490, utilityOpen: true }),
    );
    expect(loadWorkbenchLayout()).toEqual({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 280,
      utilityWidth: 440,
      utilityOpen: false,
    });
  });

  it('falls back to defaults for malformed or wrong-version values', () => {
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, '{not json');
    expect(loadWorkbenchLayout().expandedNavigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    window.localStorage.clear();
    window.localStorage.setItem(WORKBENCH_STORAGE_KEY, JSON.stringify({ version: 3 }));
    expect(loadWorkbenchLayout().expandedNavigatorWidth).toBe(DEFAULT_NAVIGATOR_WIDTH);
    expect(loadWorkbenchLayout().utilityWidth).toBe(320);
    expect(loadWorkbenchLayout().utilityOpen).toBe(false);
  });

  it('preserves an already persisted utility-open preference during a width update', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 300,
        utilityWidth: 320,
        utilityOpen: true,
      }),
    );

    expect(updateWorkbenchPreferences({ utilityWidth: 400 })).toEqual({
      version: 5,
      navigatorPreference: 'expanded',
      expandedNavigatorWidth: 300,
      utilityWidth: 400,
      utilityOpen: true,
    });
  });

  it('preserves persisted geometry during a utility-open update', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'collapsed',
        expandedNavigatorWidth: 310,
        utilityWidth: 390,
        utilityOpen: false,
      }),
    );

    expect(updateWorkbenchPreferences({ utilityOpen: true })).toEqual({
      version: 5,
      navigatorPreference: 'collapsed',
      expandedNavigatorWidth: 310,
      utilityWidth: 390,
      utilityOpen: true,
    });
  });

  it('accounts for padding, separators, and the minimum track browser width', () => {
    expect(getNavigatorWidthMax(900)).toBe(340);
    expect(getNavigatorWidthMax(1200, 340)).toBe(340);
    expect(getNavigatorWidthMax(920, 320)).toBe(MIN_NAVIGATOR_WIDTH);
    expect(getUtilityWidthMax(920)).toBe(412);
    const minimum = solveWorkbenchGeometry({
      viewportWidth: 800,
      navigatorWidth: COMPACT_NAVIGATOR_WIDTH,
      utilityWidth: getUtilityWidthMax(800),
    });
    expect(minimum.trackBrowserWidth).toBe(420);
    expect(minimum.fits).toBe(true);
    expect(COMPACT_NAVIGATOR_WIDTH).toBe(56);
    expect(getCompactNavigatorWidth('graphite')).toBe(48);
    expect(getCompactNavigatorWidth('aurora')).toBe(56);
    expect(getNavigatorWidthMax(920)).toBeGreaterThanOrEqual(MIN_NAVIGATOR_WIDTH);
  });

  it('uses the Graphite compact rail width in inspector limits and preserves legacy width', () => {
    const legacyPalettes = ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic'] as const;
    for (const palette of legacyPalettes) expect(getCompactNavigatorWidth(palette)).toBe(56);

    expect(getUtilityWidthMax(800, 'graphite')).toBe(316);
    const graphite = solveWorkbenchGeometry({
      viewportWidth: 800,
      navigatorWidth: getCompactNavigatorWidth('graphite'),
      utilityWidth: getUtilityWidthMax(800, 'graphite'),
      palette: 'graphite',
    });
    expect(graphite).toMatchObject({ navigatorWidth: 48, trackBrowserWidth: 420, fits: true });
  });

  it('automatically collapses at 640 without persisting or changing the saved preference', () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 286,
        utilityWidth: 320,
        utilityOpen: false,
      }),
    );
    const write = vi.spyOn(Storage.prototype, 'setItem');
    const { rerender } = render(<ResponsiveNavigatorHarness viewportWidth={640} />);

    expect(screen.getByTestId('responsive-navigator')).toHaveTextContent('true:48:expanded:286');
    expect(write).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Expand navigator' }));
    expect(screen.getByTestId('responsive-navigator')).toHaveTextContent('false:208:expanded:286');
    expect(write).not.toHaveBeenCalled();

    rerender(<ResponsiveNavigatorHarness viewportWidth={800} viewportRevision={1} />);
    expect(screen.getByTestId('responsive-navigator')).toHaveTextContent('false:286:expanded:286');
    expect(write).not.toHaveBeenCalled();

    rerender(<ResponsiveNavigatorHarness viewportWidth={640} viewportRevision={2} />);
    expect(screen.getByTestId('responsive-navigator')).toHaveTextContent('true:48:expanded:286');
    expect(write).not.toHaveBeenCalled();
    expect(loadWorkbenchLayout().navigatorPreference).toBe('expanded');
    expect(GRAPHITE_AUTO_COLLAPSE_NAVIGATOR_MAX_WIDTH).toBe(640);
  });

  it('uses Graphite spacing in layout math and preserves legacy palette geometry', () => {
    const legacyPalettes = ['aurora', 'minimalist', 'slate', 'cyberpunk', 'classic'] as const;
    expect(getWorkbenchGutter('graphite')).toBe(4);
    for (const palette of legacyPalettes) expect(getWorkbenchGutter(palette)).toBe(8);

    const graphite = solveWorkbenchGeometry({
      viewportWidth: 1366,
      navigatorWidth: 240,
      utilityWidth: 320,
      palette: 'graphite',
    });
    expect(graphite).toMatchObject({
      outerInlinePadding: 8,
      gutter: 4,
      separatorSpace: 8,
      navigatorWidth: 240,
      trackBrowserWidth: 790,
      utilityWidth: 320,
      fits: true,
    });

    const legacy = solveWorkbenchGeometry({
      viewportWidth: 1366,
      navigatorWidth: 240,
      utilityWidth: 320,
      palette: 'slate',
    });
    expect(legacy).toMatchObject({
      outerInlinePadding: 16,
      gutter: 8,
      separatorSpace: 16,
      trackBrowserWidth: 774,
      fits: true,
    });
    expect(getUtilityWidthMax(764, 'graphite')).toBe(280);
    expect(getUtilityWidthMax(763, 'graphite')).toBe(279);
    expect(getUtilityPresentationMode(764, true, 'graphite')).toBe('docked');
    expect(getUtilityPresentationMode(763, true, 'graphite')).toBe('overlay');
    expect(getUtilityPresentationMode(788, true, 'slate')).toBe('docked');
    expect(getUtilityPresentationMode(787, true, 'slate')).toBe('overlay');
  });

  it('chooses docked presentation only when the minimum inspector width fits', () => {
    for (const viewportWidth of [1920, 1366, 1024, 900, 800, 788]) {
      expect(getUtilityPresentationMode(viewportWidth, true)).toBe('docked');
    }
    for (const viewportWidth of [787, 780, 640]) {
      expect(getUtilityPresentationMode(viewportWidth, true)).toBe('overlay');
    }
    expect(getUtilityPresentationMode(640, false)).toBe('closed');
    expect(getUtilityWidthMax(788)).toBe(280);
    expect(getUtilityWidthMax(787)).toBe(279);
  });

  it('computes a compact overlay width independently from docked width preferences', () => {
    expect(getUtilityOverlayWidth(640)).toBe(320);
    expect(getUtilityOverlayWidth(780)).toBe(390);
    expect(getUtilityOverlayWidth(400)).toBe(280);
    expect(getUtilityOverlayWidth(250)).toBe(234);
    expect(getUtilityOverlayWidth(640, 'graphite')).toBe(288);
    expect(getUtilityOverlayWidth(800, 'graphite')).toBe(352);
    expect(getUtilityOverlayWidth(400, 'graphite')).toBe(288);
    expect(getUtilityOverlayWidth(250, 'graphite')).toBe(242);
    expect(getUtilityOverlayWidth(640, 'slate')).toBe(320);
    expect(getUtilityOverlayWidth(780, 'slate')).toBe(390);
  });

  it('combines rapid functional updates while persisting only explicit commits', () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    render(
      <StrictMode>
        <LayoutUpdateHarness />
      </StrictMode>,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Update layout' }));

    expect(screen.getByTestId('layout-values')).toHaveTextContent('300:400');
    expect(setItem).not.toHaveBeenCalled();
    expect(window.localStorage.getItem(WORKBENCH_STORAGE_KEY)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Commit utility width' }));

    expect(JSON.parse(window.localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      expandedNavigatorWidth: 300,
      utilityWidth: 410,
    });
    expect(setItem).toHaveBeenCalledTimes(1);
  });
});
