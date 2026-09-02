import { useEffect, useState, type RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { DiagnosticsPanel } from '../diagnostics/DiagnosticsPanel';
import { LibraryPanel } from '../library/LibraryPanel';
import { SongInspector } from '../inspector/SongInspector';
import { ResizableSeparator } from './ResizableSeparator';
import { useWorkbenchLayout } from './useWorkbenchLayout';
import { WorkbenchPane } from './WorkbenchPane';

interface WorkbenchProps {
  useStore: DesktopStoreHook;
  diagnosticsTriggerRef?: RefObject<HTMLButtonElement | null>;
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

export function Workbench({ useStore, diagnosticsTriggerRef }: WorkbenchProps) {
  const viewportWidth = useViewportWidth();
  const diagnosticsOpen = useStore((store) => store.diagnostics.open);
  const utilityVisible = diagnosticsOpen && viewportWidth >= 1280;
  const layout = useWorkbenchLayout(viewportWidth, utilityVisible);

  return (
    <main className={`workbench${utilityVisible ? ' has-utility' : ''}`}>
      <WorkbenchPane
        className="library-workbench-pane"
        style={{ flex: `0 0 ${layout.libraryWidth}px` }}
      >
        <LibraryPanel useStore={useStore} />
      </WorkbenchPane>
      <ResizableSeparator
        label="Resize library pane"
        value={layout.libraryWidth}
        min={280}
        max={layout.libraryMax}
        defaultValue={344}
        onChange={(value) => layout.setLibraryWidth(value)}
        onCommit={(value) => layout.setLibraryWidth(value, true)}
      />
      <WorkbenchPane className="inspector-workbench-pane">
        <SongInspector useStore={useStore} />
      </WorkbenchPane>
      {utilityVisible && (
        <>
          <ResizableSeparator
            label="Resize diagnostics pane"
            value={layout.utilityWidth}
            min={300}
            max={480}
            defaultValue={340}
            direction={-1}
            onChange={(value) => layout.setUtilityWidth(value)}
            onCommit={(value) => layout.setUtilityWidth(value, true)}
          />
          <WorkbenchPane
            className="utility-workbench-pane"
            style={{ flex: `0 0 ${layout.utilityWidth}px` }}
          >
            <DiagnosticsPanel useStore={useStore} mode="pane" />
          </WorkbenchPane>
        </>
      )}
      {diagnosticsOpen && !utilityVisible && (
        <DiagnosticsPanel
          useStore={useStore}
          mode="overlay"
          {...(diagnosticsTriggerRef ? { restoreFocusRef: diagnosticsTriggerRef } : {})}
        />
      )}
    </main>
  );
}
