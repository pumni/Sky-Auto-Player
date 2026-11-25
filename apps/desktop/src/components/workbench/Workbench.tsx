import {
  type CSSProperties,
  type RefObject,
  type TransitionEventHandler,
  useCallback,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { LibraryNavigator } from '../library/LibraryNavigator';
import { TrackBrowser } from '../tracks/TrackBrowser';
import { UtilityPane } from '../utility/UtilityPane';
import { ResizableSeparator, type ResizableSeparatorDragResult } from './ResizableSeparator';
import {
  COMPACT_NAVIGATOR_WIDTH,
  COMPACT_UTILITY_WIDTH,
  DEFAULT_NAVIGATOR_WIDTH,
  DEFAULT_UTILITY_WIDTH,
  MIN_NAVIGATOR_WIDTH,
  MIN_UTILITY_WIDTH,
  useWorkbenchLayout,
} from './useWorkbenchLayout';
import { WorkbenchPane } from './WorkbenchPane';
import {
  NAVIGATOR_COLLAPSE_THRESHOLD,
  NAVIGATOR_EXPAND_THRESHOLD,
  resolvePaneDrag,
  UTILITY_COLLAPSE_THRESHOLD,
  UTILITY_EXPAND_THRESHOLD,
} from './workbenchDragPolicy';
import {
  getAvailableNavigatorWidth,
  getNavigatorWidthMax,
  getUtilityOverlayWidth,
  getUtilityPresentationMode,
  getUtilityWidthMax,
  MIN_TRACK_BROWSER_WIDTH,
} from './workbenchGeometry';

interface WorkbenchProps {
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

interface WorkbenchMeasurement {
  width: number;
  revision: number;
}

interface LiveResizeSession {
  pane: 'navigator' | 'utility';
  rawWidth: number;
  navigator: number;
  navigatorCollapsed: boolean;
  utility: number;
  utilityCollapsed: boolean;
  lockedUtilityTopology: boolean;
}

function useWorkbenchWidth(workbenchRef: RefObject<HTMLElement | null>): WorkbenchMeasurement {
  const [measurement, setMeasurement] = useState<WorkbenchMeasurement>(() => ({
    width: window.innerWidth,
    revision: 0,
  }));
  useEffect(() => {
    const workbench = workbenchRef.current;
    if (!workbench) return undefined;

    const update = () => {
      const measuredWidth = Math.floor(workbench.getBoundingClientRect().width);
      const width = measuredWidth > 0 ? measuredWidth : window.innerWidth;
      setMeasurement((previous) =>
        previous.width === width ? previous : { width, revision: previous.revision + 1 },
      );
    };
    update();
    window.addEventListener('resize', update);

    if (typeof ResizeObserver !== 'undefined') {
      const observer = new ResizeObserver(update);
      observer.observe(workbench);
      return () => {
        observer.disconnect();
        window.removeEventListener('resize', update);
      };
    }

    return () => window.removeEventListener('resize', update);
  }, [workbenchRef]);
  return measurement;
}

export function Workbench({ utilityTriggerRef, useStore }: WorkbenchProps) {
  const workbenchRef = useRef<HTMLElement>(null);
  const navigatorSeparatorRef = useRef<HTMLDivElement>(null);
  const utilitySeparatorRef = useRef<HTMLDivElement>(null);
  const viewport = useWorkbenchWidth(workbenchRef);
  const viewportWidth = viewport.width;
  const utility = useStore((store) => store.utility);
  const presentationMode = getUtilityPresentationMode(viewportWidth, utility.open);
  const utilityDocked = presentationMode === 'docked';
  const [utilityOverlayMounted, setUtilityOverlayMounted] = useState(
    utility.open && presentationMode === 'overlay',
  );
  useEffect(() => {
    return useStore.subscribe((state, previous) => {
      if (!previous.utility.open && state.utility.open) {
        setUtilityOverlayMounted(false);
        return;
      }
      if (previous.utility.open && !state.utility.open) {
        const shouldAnimateExit =
          presentationMode === 'overlay' &&
          !window.matchMedia('(prefers-reduced-motion: reduce)').matches;
        setUtilityOverlayMounted(shouldAnimateExit);
      }
    });
  }, [presentationMode, useStore]);
  const utilityOverlayPresented =
    presentationMode === 'overlay' || (!utility.open && utilityOverlayMounted);
  const shouldRenderUtility = utility.open || (!utility.open && utilityOverlayMounted);
  const utilityOverlayWidth = getUtilityOverlayWidth(viewportWidth);
  const layout = useWorkbenchLayout(viewportWidth, utilityDocked, viewport.revision);
  const previousUtilityOpenRef = useRef(utility.open);
  const liveResizeRef = useRef<LiveResizeSession | null>(null);

  const finishUtilityExit = useCallback(() => {
    if (useStore.getState().utility.open || !utilityOverlayMounted) return;
    setUtilityOverlayMounted(false);
  }, [utilityOverlayMounted, useStore]);
  const handleUtilityTransitionEnd: TransitionEventHandler<HTMLDivElement> = (event) => {
    if (event.target !== event.currentTarget || event.propertyName !== 'opacity') return;
    finishUtilityExit();
  };
  const handleUtilityTransitionCancel: TransitionEventHandler<HTMLDivElement> = (event) => {
    if (event.target !== event.currentTarget) return;
    finishUtilityExit();
  };

  const applyLiveRawDrag = (
    pane: 'navigator' | 'utility',
    rawWidth: number,
    rebase = false,
  ): ResizableSeparatorDragResult => {
    const live = liveResizeRef.current;
    const shell = workbenchRef.current;
    if (!live || live.pane !== pane || !shell) {
      return { value: pane === 'utility' ? layout.utilityWidth : layout.navigatorWidth };
    }
    live.rawWidth = rawWidth;

    if (pane === 'navigator') {
      const utilityWidth = layout.utilityWidth;
      const navigatorMax = getNavigatorWidthMax(viewportWidth, utilityWidth);
      const canExpand =
        getAvailableNavigatorWidth(viewportWidth, utilityWidth) >= MIN_NAVIGATOR_WIDTH;
      const resolved = resolvePaneDrag({
        currentCollapsed: live.navigatorCollapsed,
        rawWidth,
        compactWidth: COMPACT_NAVIGATOR_WIDTH,
        minExpandedWidth: MIN_NAVIGATOR_WIDTH,
        maxExpandedWidth: navigatorMax,
        collapseThreshold: NAVIGATOR_COLLAPSE_THRESHOLD,
        expandThreshold: NAVIGATOR_EXPAND_THRESHOLD,
        canExpand,
      });

      if (resolved.collapsed !== live.navigatorCollapsed) {
        live.navigatorCollapsed = resolved.collapsed;
        layout.setNavigatorPreference(resolved.collapsed ? 'collapsed' : 'expanded', false);
      }

      if (rebase || live.navigator !== resolved.width) {
        live.navigator = resolved.width;
        shell
          .querySelector<HTMLElement>('[data-pane="navigator"]')
          ?.style.setProperty('--navigator-pane-width', `${resolved.width}px`);
        navigatorSeparatorRef.current?.setAttribute('aria-valuemax', String(navigatorMax));
      }
      return { value: resolved.width, snapHint: resolved.snapHint };
    }

    const utilityMax = live.utilityCollapsed
      ? getUtilityWidthMax(viewportWidth)
      : layout.utilityWidthMax;
    const minUtility = Math.min(MIN_UTILITY_WIDTH, utilityMax);
    const canExpandUtility =
      utilityDocked && utilityMax >= minUtility && minUtility > COMPACT_UTILITY_WIDTH;
    const resolved = resolvePaneDrag({
      currentCollapsed: live.utilityCollapsed,
      rawWidth,
      compactWidth: COMPACT_UTILITY_WIDTH,
      minExpandedWidth: minUtility,
      maxExpandedWidth: utilityMax,
      collapseThreshold: UTILITY_COLLAPSE_THRESHOLD,
      expandThreshold: UTILITY_EXPAND_THRESHOLD,
      canExpand: canExpandUtility,
    });

    if (resolved.collapsed !== live.utilityCollapsed) {
      live.utilityCollapsed = resolved.collapsed;
      layout.setUtilityPreference(resolved.collapsed ? 'collapsed' : 'expanded', false);
    }

    const navigatorMax = getNavigatorWidthMax(viewportWidth, resolved.width);
    const navigatorCanFit =
      getAvailableNavigatorWidth(viewportWidth, resolved.width) >= MIN_NAVIGATOR_WIDTH;
    const effectiveNavigatorCollapsed =
      layout.navigatorCollapsed || (!live.lockedUtilityTopology && !navigatorCanFit);
    const navigatorWidth = effectiveNavigatorCollapsed
      ? COMPACT_NAVIGATOR_WIDTH
      : Math.min(navigatorMax, Math.max(MIN_NAVIGATOR_WIDTH, layout.expandedNavigatorWidth));

    if (rebase || live.utility !== resolved.width || live.navigator !== navigatorWidth) {
      live.utility = resolved.width;
      live.navigator = navigatorWidth;
      shell
        .querySelector<HTMLElement>('[data-pane="inspector"]')
        ?.style.setProperty('--utility-pane-width', `${resolved.width}px`);
      shell
        .querySelector<HTMLElement>('[data-pane="navigator"]')
        ?.style.setProperty('--navigator-pane-width', `${navigatorWidth}px`);
      navigatorSeparatorRef.current?.setAttribute('aria-valuenow', String(navigatorWidth));
      navigatorSeparatorRef.current?.setAttribute('aria-valuemax', String(navigatorMax));
    }

    return { value: resolved.width, snapHint: resolved.snapHint };
  };

  const beginLiveResize = (pane: 'navigator' | 'utility') => {
    const lockUtility = pane === 'utility' && !layout.utilityCollapsed;
    liveResizeRef.current = {
      pane,
      rawWidth: pane === 'utility' ? layout.utilityWidth : layout.navigatorWidth,
      navigator: layout.navigatorWidth,
      navigatorCollapsed: layout.navigatorCollapsed,
      utility: layout.utilityWidth,
      utilityCollapsed: layout.utilityCollapsed,
      lockedUtilityTopology: lockUtility,
    };
    if (lockUtility) layout.beginUtilityDrag();
  };

  const commitLiveResize = (pane: 'navigator' | 'utility', fallbackValue: number) => {
    const live = liveResizeRef.current;
    if (pane === 'navigator') {
      if (live && live.pane === 'navigator') {
        if (live.navigatorCollapsed) {
          layout.setNavigatorPreference('collapsed', true);
        } else {
          layout.setNavigatorWidth(live.navigator, true);
        }
        return;
      }
      if (fallbackValue <= COMPACT_NAVIGATOR_WIDTH) {
        layout.setNavigatorPreference('collapsed', true);
      } else {
        layout.setNavigatorWidth(Math.max(MIN_NAVIGATOR_WIDTH, fallbackValue), true);
      }
      return;
    }

    if (live && live.pane === 'utility') {
      if (live.utilityCollapsed) {
        layout.setUtilityPreference('collapsed', true);
      } else {
        layout.setUtilityWidth(live.utility, true);
      }
      return;
    }

    if (fallbackValue <= COMPACT_UTILITY_WIDTH) {
      layout.setUtilityPreference('collapsed', true);
    } else {
      layout.setUtilityWidth(
        Math.max(Math.min(MIN_UTILITY_WIDTH, layout.utilityWidthMax), fallbackValue),
        true,
      );
    }
  };

  const endLiveResize = (pane: 'navigator' | 'utility') => {
    const live = liveResizeRef.current;
    liveResizeRef.current = null;
    if (pane === 'utility' && live?.lockedUtilityTopology && !live.utilityCollapsed) {
      layout.endUtilityDrag();
    }
  };

  const rebaseLiveResize = useEffectEvent(() => {
    const live = liveResizeRef.current;
    if (!live) return;
    if (live.pane === 'utility' && !utilityDocked) {
      liveResizeRef.current = null;
      const workbench = workbenchRef.current;
      workbench
        ?.querySelector<HTMLElement>('[data-pane="navigator"]')
        ?.style.setProperty('--navigator-pane-width', `${layout.navigatorWidth}px`);
      workbench
        ?.querySelector<HTMLElement>('[data-pane="inspector"]')
        ?.style.setProperty('--utility-pane-width', `${layout.utilityWidth}px`);
      navigatorSeparatorRef.current?.setAttribute('aria-valuenow', String(layout.navigatorWidth));
      return;
    }
    const resolved = applyLiveRawDrag(live.pane, live.rawWidth, true);
    if (live.pane === 'utility') {
      utilitySeparatorRef.current?.setAttribute('aria-valuenow', String(resolved.value));
    } else {
      navigatorSeparatorRef.current?.setAttribute('aria-valuenow', String(resolved.value));
    }
  });

  useLayoutEffect(() => {
    rebaseLiveResize();
  }, [
    viewportWidth,
    viewport.revision,
    utilityDocked,
    layout.utilityWidthMax,
    layout.utilityWidth,
    layout.utilityCollapsed,
    layout.navigatorWidth,
    layout.navigatorCollapsed,
    layout.expandedNavigatorWidth,
  ]);

  useEffect(() => {
    const openedFromClosed = utility.open && !previousUtilityOpenRef.current;
    previousUtilityOpenRef.current = utility.open;
    if (!openedFromClosed || presentationMode !== 'overlay') return;

    const frame = window.requestAnimationFrame(() => {
      const surface = workbenchRef.current?.querySelector<HTMLElement>(
        'section[role="region"][aria-label^="Utility:"]',
      );
      const activeTab = surface?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
      const firstControl = surface?.querySelector<HTMLElement>(
        'button:not([disabled]), [tabindex="0"]',
      );
      (activeTab ?? firstControl ?? surface)?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [presentationMode, utility.open]);

  const navigatorSeparatorDisabled = layout.navigatorCollapsed && !layout.canExpandNavigator;

  return (
    <main
      ref={workbenchRef}
      className="relative flex min-h-0 min-w-0 gap-0 overflow-hidden px-1 pb-1"
      aria-label="Music sheet workbench"
      data-slot="workbench"
      data-layout-fits={layout.geometry.fits ? 'true' : 'false'}
      data-utility-presentation={presentationMode}
      data-workbench-gap={layout.geometry.gutter}
      data-workbench-width={viewportWidth}
      style={
        {
          '--min-track-browser-width': `${MIN_TRACK_BROWSER_WIDTH}px`,
        } as CSSProperties
      }
    >
      <WorkbenchPane
        pane="navigator"
        collapsed={layout.navigatorCollapsed}
        className="shrink-0 bg-card"
        style={
          {
            '--navigator-pane-width': `${layout.navigatorWidth}px`,
            flex: '0 0 var(--navigator-pane-width)',
          } as CSSProperties
        }
      >
        <LibraryNavigator
          collapsed={layout.navigatorCollapsed}
          onToggleCollapsed={() =>
            layout.setNavigatorPreference(
              layout.navigatorCollapsed ? 'expanded' : 'collapsed',
              true,
            )
          }
          useStore={useStore}
        />
      </WorkbenchPane>
      <ResizableSeparator
        ref={navigatorSeparatorRef}
        label="Resize library navigator"
        value={layout.navigatorWidth}
        min={layout.navigatorCollapsed ? COMPACT_NAVIGATOR_WIDTH : MIN_NAVIGATOR_WIDTH}
        max={layout.navigatorMax}
        defaultValue={DEFAULT_NAVIGATOR_WIDTH}
        disabled={navigatorSeparatorDisabled}
        onDragStart={() => beginLiveResize('navigator')}
        onDragRaw={(raw) => applyLiveRawDrag('navigator', raw)}
        onDragEnd={() => endLiveResize('navigator')}
        onChange={(value) => {
          if (value <= COMPACT_NAVIGATOR_WIDTH) {
            layout.setNavigatorPreference('collapsed');
          } else {
            layout.setNavigatorWidth(Math.max(MIN_NAVIGATOR_WIDTH, value));
          }
        }}
        onCommit={(value) => commitLiveResize('navigator', value)}
        onDoubleClick={() => {
          if (layout.navigatorCollapsed) {
            layout.setNavigatorPreference('expanded', true);
          } else {
            layout.resetNavigatorWidth();
          }
        }}
      />
      <WorkbenchPane
        pane="browser"
        className="min-w-[420px] flex-[1_1_0] [container-name:track-browser] [container-type:inline-size]"
        style={{ minWidth: MIN_TRACK_BROWSER_WIDTH }}
      >
        <TrackBrowser useStore={useStore} />
      </WorkbenchPane>
      {shouldRenderUtility && (
        <>
          {utilityDocked && (
            <ResizableSeparator
              ref={utilitySeparatorRef}
              label="Resize utility pane"
              value={layout.utilityWidth}
              min={
                layout.utilityCollapsed
                  ? COMPACT_UTILITY_WIDTH
                  : Math.min(MIN_UTILITY_WIDTH, layout.utilityWidthMax)
              }
              max={layout.utilityWidthMax}
              defaultValue={DEFAULT_UTILITY_WIDTH}
              direction={-1}
              onDragStart={() => beginLiveResize('utility')}
              onDragRaw={(raw) => applyLiveRawDrag('utility', raw)}
              onDragEnd={() => endLiveResize('utility')}
              onChange={(value) => {
                if (value <= COMPACT_UTILITY_WIDTH) {
                  layout.setUtilityPreference('collapsed');
                } else {
                  layout.setUtilityWidth(
                    Math.max(Math.min(MIN_UTILITY_WIDTH, layout.utilityWidthMax), value),
                  );
                }
              }}
              onCommit={(value) => commitLiveResize('utility', value)}
              onDoubleClick={() => {
                if (layout.utilityCollapsed) {
                  layout.setUtilityPreference('expanded', true);
                } else {
                  layout.resetUtilityWidth();
                }
              }}
            />
          )}
          <WorkbenchPane
            pane="inspector"
            collapsed={utilityDocked && layout.utilityCollapsed}
            presentationMode={utilityOverlayPresented ? 'overlay' : 'docked'}
            ariaHidden={!utility.open}
            inert={!utility.open}
            open={utility.open}
            onTransitionEnd={handleUtilityTransitionEnd}
            onTransitionCancel={handleUtilityTransitionCancel}
            className={`shrink-0 bg-card [container-name:utility] [container-type:inline-size] ${
              utilityOverlayPresented
                ? 'absolute top-0 right-1 bottom-1 z-20 max-w-[calc(100%-8px)] rounded-dialog border-border shadow-xl transition-[opacity,translate] duration-[220ms] ease-enter data-[open=false]:pointer-events-none data-[open=false]:translate-x-4 data-[open=false]:opacity-0 data-[open=false]:duration-[160ms] data-[open=false]:ease-exit motion-reduce:transition-none [contain:none]'
                : ''
            }`}
            style={
              utilityOverlayPresented
                ? ({
                    '--utility-pane-width': `${layout.utilityWidth}px`,
                    width: utilityOverlayWidth,
                  } as CSSProperties)
                : ({
                    '--utility-pane-width': `${layout.utilityWidth}px`,
                    flex: '0 0 var(--utility-pane-width)',
                  } as CSSProperties)
            }
          >
            <UtilityPane
              presentationMode={utilityOverlayPresented ? 'overlay' : 'docked'}
              isExiting={!utility.open && utilityOverlayPresented}
              collapsed={utilityDocked && layout.utilityCollapsed}
              onToggleCollapsed={() =>
                layout.setUtilityPreference(
                  layout.utilityCollapsed ? 'expanded' : 'collapsed',
                  true,
                )
              }
              onExpand={() => layout.setUtilityPreference('expanded', true)}
              useStore={useStore}
              utilityTriggerRef={utilityTriggerRef}
            />
          </WorkbenchPane>
        </>
      )}
    </main>
  );
}
