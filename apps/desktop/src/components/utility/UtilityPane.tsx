import { Activity, ChevronLeft, FileMusic } from 'lucide-react';
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  type ReactNode,
  type RefObject,
} from 'react';
import type { DesktopStoreHook, UtilityView } from '../../state/store';
import { Button } from '../ui/button';
import { Tabs, TabsContent } from '../ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';
import { SongDetailsView } from './SongDetailsView';
import { UtilityHeader } from './UtilityHeader';

const DiagnosticsView = lazy(() =>
  import('./DiagnosticsView').then(({ DiagnosticsView: component }) => ({ default: component })),
);

interface UtilityRailItemProps {
  label: string;
  active: boolean;
  icon: ReactNode;
  onSelect: () => void;
}

function UtilityRailItem({ label, active, icon, onSelect }: UtilityRailItemProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="relative size-10 rounded-control p-0 text-text-secondary after:absolute after:inset-y-2.5 after:right-0.5 after:w-[3px] after:rounded-full after:bg-accent-ink after:opacity-0 hover:bg-control-hover hover:text-foreground aria-pressed:bg-selection aria-pressed:text-accent-ink aria-pressed:after:opacity-100"
            type="button"
            aria-label={label}
            aria-pressed={active}
            onClick={onSelect}
          />
        }
      >
        <span className="grid place-items-center" aria-hidden="true">
          {icon}
        </span>
      </TooltipTrigger>
      <TooltipContent
        side="left"
        className="max-w-[min(320px,calc(100vw-64px))] break-words rounded-control border border-border bg-popover px-2 py-1.5 text-xs leading-[1.4] text-popover-foreground shadow-lg"
      >
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

interface UtilityPaneProps {
  presentationMode: 'docked' | 'overlay';
  isExiting?: boolean;
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  onExpand?: () => void;
  utilityTriggerRef: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function UtilityPane({
  presentationMode,
  isExiting = false,
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

  if (!utility.open && !isExiting) return null;

  const isCollapsedRail = presentationMode === 'docked' && collapsed;

  if (isCollapsedRail) {
    return (
      <section
        ref={paneRef}
        id="utility-inspector"
        className="flex min-h-0 min-w-0 flex-1 flex-col items-center bg-card px-[3px] py-2"
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
        <div className="mb-1 flex min-h-10 w-full items-center justify-center">
          <Button
            variant="ghost"
            size="icon-lg"
            ref={expandButtonRef}
            className="size-10 p-0 hover:bg-control-hover"
            type="button"
            aria-label="Expand Inspector"
            title="Expand Inspector"
            onClick={handleToggleCollapsed}
          >
            <ChevronLeft size={17} aria-hidden="true" />
          </Button>
        </div>
        <div className="grid justify-items-center gap-1" role="group" aria-label="Utility views">
          <UtilityRailItem
            label="Song Details"
            active={utility.activeView === 'details'}
            icon={<FileMusic size={18} />}
            onSelect={() => handleRailExpand('details')}
          />
          <UtilityRailItem
            label="Runtime"
            active={utility.activeView === 'diagnostics'}
            icon={<Activity size={18} />}
            onSelect={() => handleRailExpand('diagnostics')}
          />
        </div>
      </section>
    );
  }

  return (
    <section
      ref={paneRef}
      id="utility-inspector"
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-card"
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
        className="flex min-h-0 min-w-0 flex-1 flex-col gap-0"
        value={utility.activeView}
        onValueChange={(value) => {
          if (value === 'details' || value === 'diagnostics') setUtilityView(value);
        }}
      >
        <UtilityHeader
          activeView={utility.activeView}
          presentationMode={presentationMode}
          collapseButtonRef={collapseButtonRef}
          {...(onToggleCollapsed ? { onCollapse: handleToggleCollapsed } : {})}
          onClose={close}
        />
        <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
          <TabsContent className="flex min-h-0 min-w-0 flex-1 overflow-auto" value="details">
            {utility.activeView === 'details' && <SongDetailsView useStore={useStore} />}
          </TabsContent>
          <TabsContent className="flex min-h-0 min-w-0 flex-1 overflow-auto" value="diagnostics">
            {utility.activeView === 'diagnostics' && (
              <Suspense
                fallback={
                  <div
                    className="grid flex-1 place-items-center p-4 text-sm text-muted-foreground"
                    role="status"
                  >
                    Loading diagnostics…
                  </div>
                }
              >
                <DiagnosticsView useStore={useStore} />
              </Suspense>
            )}
          </TabsContent>
        </div>
      </Tabs>
    </section>
  );
}
