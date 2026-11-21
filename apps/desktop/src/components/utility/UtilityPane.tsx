import { Activity, ChevronLeft, FileMusic } from 'lucide-react';
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import { Button, Tabs, TabPanel } from 'react-aria-components';
import { Tooltip, TooltipTrigger } from 'react-aria-components/Tooltip';
import type { DesktopStoreHook, UtilityView } from '../../state/store';
import { SongDetailsView } from './SongDetailsView';
import { UtilityHeader } from './UtilityHeader';

const DiagnosticsView = lazy(() =>
  import('./DiagnosticsView').then(({ DiagnosticsView: component }) => ({ default: component })),
);

interface UtilityRailItemProps {
  label: string;
  active: boolean;
  icon: ReactNode;
  onPress: () => void;
}

function UtilityRailItem({ label, active, icon, onPress }: UtilityRailItemProps) {
  const [focusTooltipOpen, setFocusTooltipOpen] = useState(false);
  const [interactionTooltipOpen, setInteractionTooltipOpen] = useState(false);

  return (
    <TooltipTrigger
      delay={350}
      closeDelay={0}
      isOpen={focusTooltipOpen || interactionTooltipOpen}
      onOpenChange={setInteractionTooltipOpen}
    >
      <Button
        className={`utility-rail-item${active ? ' is-active' : ''}`}
        type="button"
        aria-label={label}
        aria-pressed={active}
        onFocus={() => setFocusTooltipOpen(true)}
        onBlur={() => setFocusTooltipOpen(false)}
        onPress={onPress}
      >
        <span className="utility-rail-item-icon" aria-hidden="true">
          {icon}
        </span>
      </Button>
      <Tooltip className="library-playlist-tooltip" placement="left">
        {label}
      </Tooltip>
    </TooltipTrigger>
  );
}

interface UtilityPaneProps {
  presentationMode: 'docked' | 'overlay';
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  onExpand?: () => void;
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function UtilityPane({
  presentationMode,
  collapsed = false,
  onToggleCollapsed,
  onExpand,
  utilityTriggerRef,
  useStore,
}: UtilityPaneProps) {
  const utility = useStore((store) => store.utility);
  const closeUtility = useStore((store) => store.closeUtility);
  const setUtilityView = useStore((store) => store.setUtilityView);
  const paneRef = useRef<HTMLElement>(null);
  const expandButtonRef = useRef<HTMLButtonElement>(null);
  const collapseButtonRef = useRef<HTMLButtonElement>(null);
  const pendingFocusTransition = useRef<'toCollapsed' | 'toExpanded' | null>(null);

  useLayoutEffect(() => {
    if (pendingFocusTransition.current === 'toCollapsed') {
      pendingFocusTransition.current = null;
      expandButtonRef.current?.focus();
    } else if (pendingFocusTransition.current === 'toExpanded') {
      pendingFocusTransition.current = null;
      collapseButtonRef.current?.focus();
    }
  }, [collapsed]);

  const handleToggleCollapsed = useCallback(() => {
    if (paneRef.current?.contains(document.activeElement)) {
      pendingFocusTransition.current = collapsed ? 'toExpanded' : 'toCollapsed';
    }
    onToggleCollapsed?.();
  }, [collapsed, onToggleCollapsed]);

  const handleRailExpand = useCallback(
    (targetView: UtilityView) => {
      if (paneRef.current?.contains(document.activeElement)) {
        pendingFocusTransition.current = 'toExpanded';
      }
      setUtilityView(targetView);
      onExpand?.();
    },
    [onExpand, setUtilityView],
  );

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

  const isCollapsedRail = presentationMode === 'docked' && collapsed;

  if (isCollapsedRail) {
    return (
      <section
        ref={paneRef}
        id="utility-inspector"
        className="utility-surface is-collapsed"
        role="region"
        tabIndex={-1}
        aria-label={`Utility: ${utility.activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details'}`}
        data-presentation-mode={presentationMode}
        data-collapsed="true"
        onKeyDown={(event) => {
          if (event.key === 'Escape' && !event.defaultPrevented) {
            event.preventDefault();
            close();
          }
        }}
      >
        <div className="utility-collapsed-header">
          <button
            ref={expandButtonRef}
            className="icon-button icon-button-boxed utility-collapse-button"
            type="button"
            aria-label="Expand Inspector"
            title="Expand Inspector"
            onClick={handleToggleCollapsed}
          >
            <ChevronLeft size={17} aria-hidden="true" />
          </button>
        </div>
        <div className="utility-collapsed-items" role="group" aria-label="Utility views">
          <UtilityRailItem
            label="Song Details"
            active={utility.activeView === 'details'}
            icon={<FileMusic size={18} />}
            onPress={() => handleRailExpand('details')}
          />
          <UtilityRailItem
            label="Runtime"
            active={utility.activeView === 'diagnostics'}
            icon={<Activity size={18} />}
            onPress={() => handleRailExpand('diagnostics')}
          />
        </div>
      </section>
    );
  }

  return (
    <section
      ref={paneRef}
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
      <Tabs
        className="utility-tabs"
        selectedKey={utility.activeView}
        onSelectionChange={(key) => {
          if (key === 'details' || key === 'diagnostics') setUtilityView(key);
        }}
      >
        <UtilityHeader
          activeView={utility.activeView}
          presentationMode={presentationMode}
          collapseButtonRef={collapseButtonRef}
          {...(onToggleCollapsed ? { onCollapse: handleToggleCollapsed } : {})}
          onClose={close}
        />
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
      </Tabs>
    </section>
  );
}
