import { useEffect, useRef, useState, type RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { LibraryNavigator } from '../library/LibraryNavigator';
import { TrackBrowser } from '../tracks/TrackBrowser';
import { UtilityPane } from '../utility/UtilityPane';
import { ResizableSeparator } from './ResizableSeparator';
import {
  getUtilityOverlayWidth,
  getUtilityPresentationMode,
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
    >
      <WorkbenchPane
        className={`navigator-workbench-pane${layout.navigatorCollapsed ? ' is-collapsed' : ''}`}
        style={{ flex: `0 0 ${layout.navigatorWidth}px` }}
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
              onDragStart={layout.beginUtilityDrag}
              onDragEnd={layout.endUtilityDrag}
              onChange={(value) => layout.setUtilityWidth(value)}
              onCommit={(value) => layout.setUtilityWidth(value, true)}
            />
          )}
          <WorkbenchPane
            className={`utility-workbench-pane${presentationMode === 'overlay' ? ' is-overlay' : ''}`}
            style={
              presentationMode === 'overlay'
                ? { width: utilityOverlayWidth }
                : { flex: `0 0 ${layout.utilityWidth}px` }
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
