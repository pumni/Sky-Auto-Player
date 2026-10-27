import {
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { LibraryNavigator } from '../library/LibraryNavigator';
import { TrackBrowser } from '../tracks/TrackBrowser';
import { UtilityPane } from '../utility/UtilityPane';
import { ResizableSeparator } from './ResizableSeparator';
import {
  getUtilityOverlayWidth,
  getUtilityPresentationMode,
  getNavigatorWidthMax,
  useWorkbenchLayout,
} from './useWorkbenchLayout';
import { WorkbenchPane } from './WorkbenchPane';

interface WorkbenchProps {
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

interface WorkbenchMeasurement {
  width: number;
  revision: number;
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
  const viewport = useWorkbenchWidth(workbenchRef);
  const viewportWidth = viewport.width;
  const utility = useStore((store) => store.utility);
  const palette = useStore(
    (store) => store.settings?.palette ?? store.bootstrap?.settings.palette ?? 'graphite',
  );
  const presentationMode = getUtilityPresentationMode(viewportWidth, utility.open, palette);
  const utilityDocked = presentationMode === 'docked';
  const utilityOverlayWidth = getUtilityOverlayWidth(viewportWidth, palette);
  const layout = useWorkbenchLayout(viewportWidth, utilityDocked, palette, viewport.revision);
  const previousUtilityOpenRef = useRef(utility.open);
  const liveResizeRef = useRef<{
    pane: 'navigator' | 'utility';
    width: number;
    navigator: number;
    utility: number;
  } | null>(null);

  const applyLiveResize = (pane: 'navigator' | 'utility', width: number, rebase = false) => {
    const live = liveResizeRef.current;
    const shell = workbenchRef.current;
    if (!live || live.pane !== pane || !shell) return;
    const utilityWidth =
      pane === 'utility'
        ? Math.min(layout.utilityWidthMax, Math.max(Math.min(280, layout.utilityWidthMax), width))
        : layout.utilityWidth;
    const navigatorMax = getNavigatorWidthMax(viewportWidth, utilityWidth, palette);
    const navigatorWidth = layout.navigatorCollapsed
      ? layout.navigatorWidth
      : Math.min(
          navigatorMax,
          Math.max(200, pane === 'navigator' ? width : layout.expandedNavigatorWidth),
        );
    live.width = pane === 'utility' ? utilityWidth : navigatorWidth;
    if (!rebase && live.navigator === navigatorWidth && live.utility === utilityWidth) return;
    live.navigator = navigatorWidth;
    live.utility = utilityWidth;
    shell.style.setProperty('--navigator-pane-width', `${navigatorWidth}px`);
    shell.style.setProperty('--utility-pane-width', `${utilityWidth}px`);
    const navigator = shell.querySelector('[aria-label="Resize library navigator"]');
    navigator?.setAttribute('aria-valuenow', String(navigatorWidth));
    navigator?.setAttribute('aria-valuemax', String(navigatorMax));
    shell
      .querySelector('[aria-label="Resize utility pane"]')
      ?.setAttribute('aria-valuenow', String(utilityWidth));
  };

  const beginLiveResize = (pane: 'navigator' | 'utility') => {
    liveResizeRef.current = {
      pane,
      width: pane === 'utility' ? layout.utilityWidth : layout.navigatorWidth,
      navigator: layout.navigatorWidth,
      utility: layout.utilityWidth,
    };
    if (pane === 'utility') layout.beginUtilityDrag();
  };

  const rebaseLiveResize = useEffectEvent(() => {
    const live = liveResizeRef.current;
    if (!live) return;
    if (live.pane === 'utility' && !utilityDocked) {
      liveResizeRef.current = null;
      // Discard an uncommitted session without leaving an imperative width on
      // a pane whose React style value may not otherwise have changed.
      workbenchRef.current?.style.setProperty(
        '--navigator-pane-width',
        `${layout.navigatorWidth}px`,
      );
      workbenchRef.current?.style.setProperty('--utility-pane-width', `${layout.utilityWidth}px`);
      workbenchRef.current
        ?.querySelector('[aria-label="Resize library navigator"]')
        ?.setAttribute('aria-valuenow', String(layout.navigatorWidth));
      return;
    }
    applyLiveResize(live.pane, live.width, true);
  });
  useLayoutEffect(() => {
    rebaseLiveResize();
  }, [
    viewportWidth,
    viewport.revision,
    palette,
    utilityDocked,
    layout.utilityWidthMax,
    layout.utilityWidth,
    layout.navigatorWidth,
    layout.navigatorCollapsed,
    layout.expandedNavigatorWidth,
  ]);

  useEffect(() => {
    const openedFromClosed = utility.open && !previousUtilityOpenRef.current;
    previousUtilityOpenRef.current = utility.open;
    if (!openedFromClosed || presentationMode !== 'overlay') return;

    const frame = window.requestAnimationFrame(() => {
      const surface = workbenchRef.current?.querySelector<HTMLElement>('.utility-surface');
      const activeTab = surface?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
      const firstControl = surface?.querySelector<HTMLElement>(
        'button:not([disabled]), [tabindex="0"]',
      );
      (activeTab ?? firstControl ?? surface)?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [presentationMode, utility.open]);

  return (
    <main
      ref={workbenchRef}
      className={`workbench${utility.open ? ' has-utility' : ''}`}
      aria-label="Music sheet workbench"
      data-layout-fits={layout.geometry.fits ? 'true' : 'false'}
      data-utility-presentation={presentationMode}
      data-workbench-gap={layout.geometry.gutter}
      data-workbench-width={viewportWidth}
      style={
        {
          '--navigator-pane-width': `${layout.navigatorWidth}px`,
          '--utility-pane-width': `${layout.utilityWidth}px`,
        } as CSSProperties
      }
    >
      <WorkbenchPane
        className={`navigator-workbench-pane${layout.navigatorCollapsed ? ' is-collapsed' : ''}`}
        style={{ flex: '0 0 var(--navigator-pane-width)' }}
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
        label="Resize library navigator"
        value={layout.navigatorWidth}
        min={layout.navigatorCollapsed ? layout.navigatorWidth : 200}
        max={layout.navigatorMax}
        defaultValue={240}
        disabled={layout.navigatorCollapsed}
        onDragStart={() => beginLiveResize('navigator')}
        onDragChange={(value) => applyLiveResize('navigator', value)}
        onDragEnd={() => {
          liveResizeRef.current = null;
        }}
        onChange={(value) => layout.setNavigatorWidth(value)}
        onCommit={(value) => layout.setNavigatorWidth(value, true)}
      />
      <WorkbenchPane className="track-browser-workbench-pane">
        <TrackBrowser useStore={useStore} />
      </WorkbenchPane>
      {utility.open && (
        <>
          {utilityDocked && (
            <ResizableSeparator
              label="Resize utility pane"
              value={layout.utilityWidth}
              min={Math.min(280, layout.utilityWidth)}
              max={layout.utilityWidthMax}
              defaultValue={320}
              direction={-1}
              onDragStart={() => beginLiveResize('utility')}
              onDragChange={(value) => applyLiveResize('utility', value)}
              onDragEnd={() => {
                liveResizeRef.current = null;
                layout.endUtilityDrag();
              }}
              onChange={(value) => layout.setUtilityWidth(value)}
              onCommit={(value) => layout.setUtilityWidth(value, true)}
            />
          )}
          <WorkbenchPane
            className={`utility-workbench-pane${presentationMode === 'overlay' ? ' is-overlay' : ''}`}
            style={
              presentationMode === 'overlay'
                ? { width: utilityOverlayWidth }
                : { flex: '0 0 var(--utility-pane-width)' }
            }
          >
            <UtilityPane
              presentationMode={utilityDocked ? 'docked' : 'overlay'}
              useStore={useStore}
              utilityTriggerRef={utilityTriggerRef}
            />
          </WorkbenchPane>
        </>
      )}
    </main>
  );
}
