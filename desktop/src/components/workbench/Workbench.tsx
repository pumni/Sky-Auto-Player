import { useEffect, useState, type RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { LibraryNavigator } from '../library/LibraryNavigator';
import { TrackBrowser } from '../tracks/TrackBrowser';
import { UtilityPane } from '../utility/UtilityPane';
import { ResizableSeparator } from './ResizableSeparator';
import { useWorkbenchLayout } from './useWorkbenchLayout';
import { WorkbenchPane } from './WorkbenchPane';

interface WorkbenchProps {
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
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

export function Workbench({ utilityTriggerRef, useStore }: WorkbenchProps) {
  const viewportWidth = useViewportWidth();
  const utility = useStore((store) => store.utility);
  const layout = useWorkbenchLayout(viewportWidth, utility.open);

  return (
    <main
      className={`workbench${utility.open ? ' has-utility' : ''}`}
      aria-label="Music sheet workbench"
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
        min={layout.navigatorCollapsed ? layout.navigatorWidth : 220}
        max={layout.navigatorMax}
        defaultValue={260}
        disabled={layout.navigatorCollapsed}
        onChange={(value) => layout.setNavigatorWidth(value)}
        onCommit={(value) => layout.setNavigatorWidth(value, true)}
      />
      <WorkbenchPane className="track-browser-workbench-pane">
        <TrackBrowser useStore={useStore} />
      </WorkbenchPane>
      {utility.open && (
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
            <UtilityPane useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
          </WorkbenchPane>
        </>
      )}
    </main>
  );
}
