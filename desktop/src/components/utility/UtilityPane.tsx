import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  type ComponentProps,
  type ComponentType,
  type RefObject,
} from 'react';
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
  presentationMode: 'docked' | 'overlay';
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function UtilityPane({ presentationMode, utilityTriggerRef, useStore }: UtilityPaneProps) {
  const utility = useStore((store) => store.utility);
  const closeUtility = useStore((store) => store.closeUtility);
  const setUtilityView = useStore((store) => store.setUtilityView);
  const restoreFocus = useCallback(
    (previousFocus: Element | null) => {
      window.queueMicrotask(() => {
        const trigger = utilityTriggerRef.current;
        const currentFocus = document.activeElement;
        if (
          trigger?.isConnected &&
          (currentFocus === previousFocus ||
            currentFocus === document.body ||
            !currentFocus?.isConnected)
        ) {
          trigger.focus();
        }
      });
    },
    [utilityTriggerRef],
  );

  const close = useCallback(() => {
    const previousFocus = document.activeElement;
    closeUtility();
    restoreFocus(previousFocus);
  }, [closeUtility, restoreFocus]);

  useEffect(() => {
    if (presentationMode !== 'overlay' || !utility.open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      close();
    };
    document.addEventListener('keydown', closeOnEscape);
    return () => document.removeEventListener('keydown', closeOnEscape);
  }, [close, presentationMode, utility.open]);

  if (!utility.open) return null;

  return (
    <section
      id="utility-inspector"
      className="utility-surface"
      role="region"
      tabIndex={-1}
      aria-label={`Utility: ${utility.activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details'}`}
      data-presentation-mode={presentationMode}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && !event.defaultPrevented) {
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
