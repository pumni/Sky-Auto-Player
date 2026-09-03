import { Tabs, TabPanel } from 'react-aria-components';
import type { ComponentProps, ComponentType, RefObject } from 'react';
import type { DesktopStoreHook, UtilityView } from '../../state/store';
import { DiagnosticsView } from './DiagnosticsView';
import { SongDetailsView } from './SongDetailsView';
import { UtilityHeader } from './UtilityHeader';

type UtilityTabsProps = ComponentProps<typeof Tabs> & {
  defaultSelectedKey?: UtilityView;
  onSelectionChange?: (key: UtilityView) => void;
};

// react-aria-components 1.20 omits the selection props from TabsProps even
// though the runtime forwards them to useTabListState.
const UtilityTabs = Tabs as unknown as ComponentType<UtilityTabsProps>;

interface UtilityPaneProps {
  mode: 'pane' | 'overlay';
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function UtilityPane({ mode, utilityTriggerRef, useStore }: UtilityPaneProps) {
  const utility = useStore((store) => store.utility);
  const closeUtility = useStore((store) => store.closeUtility);
  const setUtilityView = useStore((store) => store.setUtilityView);
  if (!utility.open) return null;

  const restoreFocus = () => {
    window.queueMicrotask(() => utilityTriggerRef.current?.focus());
  };
  const close = () => {
    closeUtility();
    restoreFocus();
  };

  return (
    <section
      className={`utility-surface utility-${mode}`}
      role="region"
      aria-label={`Utility: ${utility.activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details'}`}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          close();
        }
      }}
    >
      <UtilityTabs
        className="utility-tabs"
        defaultSelectedKey={utility.activeView}
        onSelectionChange={(key) => {
          setUtilityView(key);
        }}
      >
        <UtilityHeader activeView={utility.activeView} onClose={close} />
        <div className="utility-content">
          <TabPanel id="details">
            <SongDetailsView useStore={useStore} />
          </TabPanel>
          <TabPanel id="diagnostics">
            <DiagnosticsView useStore={useStore} />
          </TabPanel>
        </div>
      </UtilityTabs>
    </section>
  );
}
