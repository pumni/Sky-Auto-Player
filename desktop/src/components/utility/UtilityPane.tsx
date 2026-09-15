import { lazy, Suspense, type ComponentProps, type ComponentType, type RefObject } from 'react';
import { Tabs, TabPanel } from 'react-aria-components';
import type { DesktopStoreHook, UtilityView } from '../../state/store';
import { SongDetailsView } from './SongDetailsView';
import { UtilityHeader } from './UtilityHeader';

const DiagnosticsView = lazy(() =>
  import('./DiagnosticsView').then(({ DiagnosticsView: component }) => ({ default: component })),
);

type UtilityTabsProps = ComponentProps<typeof Tabs> & {
  defaultSelectedKey?: UtilityView;
  onSelectionChange?: (key: UtilityView) => void;
};

// react-aria-components 1.20 omits the selection props from TabsProps even
// though the runtime forwards them to useTabListState.
const UtilityTabs = Tabs as unknown as ComponentType<UtilityTabsProps>;

interface UtilityPaneProps {
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function UtilityPane({ utilityTriggerRef, useStore }: UtilityPaneProps) {
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
      className="utility-surface"
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
            {utility.activeView === 'details' && <SongDetailsView useStore={useStore} />}
          </TabPanel>
          <TabPanel id="diagnostics">
            {utility.activeView === 'diagnostics' && (
              <Suspense
                fallback={
                  <div className="utility-loading" role="status">
                    Loading diagnostics…
                  </div>
                }
              >
                <DiagnosticsView useStore={useStore} />
              </Suspense>
            )}
          </TabPanel>
        </div>
      </UtilityTabs>
    </section>
  );
}
