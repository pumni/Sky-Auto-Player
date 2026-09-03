import { useEffect, useState, type RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { LibraryNavigator } from '../library/LibraryNavigator';
import { TrackBrowser } from '../tracks/TrackBrowser';
import { UtilityPane } from '../utility/UtilityPane';
import { ResizableSeparator } from './ResizableSeparator';
import { useWorkbenchLayout } from './useWorkbenchLayout';
import { WorkbenchPane } from './WorkbenchPane';

interface WorkbenchProps {
  detailsTriggerRef?: RefObject<HTMLButtonElement | null>;
  diagnosticsTriggerRef?: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

function useViewportWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return width;
}

export function Workbench({ detailsTriggerRef, diagnosticsTriggerRef, useStore }: WorkbenchProps) {
  const viewportWidth = useViewportWidth();
  const utility = useStore((store) => store.utility);
  const utilityVisible = utility.open && viewportWidth >= 1280;
  const layout = useWorkbenchLayout(viewportWidth, utilityVisible);
  const restoreFocusRef =
    utility.activeView === 'diagnostics' ? diagnosticsTriggerRef : detailsTriggerRef;

  return (
    <main
      className={`workbench${utilityVisible ? ' has-utility' : ''}${
        utility.open && !utilityVisible ? ' has-overlay-utility' : ''
      }`}
      aria-label="Music sheet workbench"
    >
      <WorkbenchPane
        className="navigator-workbench-pane"
        style={{ flex: `0 0 ${layout.navigatorWidth}px` }}
      >
        <LibraryNavigator useStore={useStore} />
      </WorkbenchPane>
      <ResizableSeparator
        label="Resize library navigator"
        value={layout.navigatorWidth}
        min={220}
        max={layout.navigatorMax}
        defaultValue={260}
        onChange={(value) => layout.setNavigatorWidth(value)}
        onCommit={(value) => layout.setNavigatorWidth(value, true)}
      />
      <WorkbenchPane className="track-browser-workbench-pane">
        <TrackBrowser detailsTriggerRef={detailsTriggerRef} useStore={useStore} />
      </WorkbenchPane>
      {utilityVisible && (
        <>
          <ResizableSeparator
            label="Resize utility pane"
            value={layout.utilityWidth}
            min={320}
            max={480}
            defaultValue={360}
            direction={-1}
            onChange={(value) => layout.setUtilityWidth(value)}
            onCommit={(value) => layout.setUtilityWidth(value, true)}
          />
          <WorkbenchPane
            className="utility-workbench-pane"
            style={{ flex: `0 0 ${layout.utilityWidth}px` }}
          >
            <UtilityPane
              useStore={useStore}
              mode="pane"
              {...(restoreFocusRef ? { restoreFocusRef } : {})}
            />
          </WorkbenchPane>
        </>
      )}
      {utility.open && !utilityVisible && (
        <UtilityPane
          useStore={useStore}
          mode="overlay"
          {...(restoreFocusRef ? { restoreFocusRef } : {})}
        />
      )}
    </main>
  );
}
