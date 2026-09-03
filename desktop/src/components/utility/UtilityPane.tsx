import { useEffect, useRef, type RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { DiagnosticsView } from './DiagnosticsView';
import { SongDetailsView } from './SongDetailsView';
import { UtilityHeader } from './UtilityHeader';

interface UtilityPaneProps {
  mode: 'pane' | 'overlay';
  restoreFocusRef?: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

function focusableElements(root: HTMLElement | null): HTMLElement[] {
  return Array.from(
    root?.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    ) ?? [],
  ).filter((element) => !element.hasAttribute('disabled'));
}

export function UtilityPane({ mode, restoreFocusRef, useStore }: UtilityPaneProps) {
  const utility = useStore((store) => store.utility);
  const closeUtility = useStore((store) => store.closeUtility);
  const setUtilityView = useStore((store) => store.setUtilityView);
  const surfaceRef = useRef<HTMLElement>(null);
  const overlay = mode === 'overlay';

  useEffect(() => {
    if (!overlay) return;
    const frame = window.requestAnimationFrame(() => surfaceRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [overlay]);

  if (!utility.open) return null;

  const restoreFocus = () => {
    window.queueMicrotask(() => restoreFocusRef?.current?.focus());
  };
  const close = () => {
    closeUtility();
    if (overlay) restoreFocus();
  };

  return (
    <section
      ref={surfaceRef}
      className={`utility-surface utility-${mode}`}
      role={overlay ? 'dialog' : 'region'}
      aria-label={utility.activeView === 'diagnostics' ? 'Diagnostics' : 'Song Details'}
      aria-modal={overlay ? true : undefined}
      tabIndex={overlay ? -1 : undefined}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          close();
          return;
        }
        if (!overlay || event.key !== 'Tab') return;
        const focusables = focusableElements(surfaceRef.current);
        if (focusables.length === 0) {
          event.preventDefault();
          surfaceRef.current?.focus();
          return;
        }
        const first = focusables[0]!;
        const last = focusables[focusables.length - 1]!;
        const active = document.activeElement;
        if (event.shiftKey && (active === first || active === surfaceRef.current)) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && active === last) {
          event.preventDefault();
          first.focus();
        }
      }}
    >
      <UtilityHeader
        activeView={utility.activeView}
        onClose={close}
        onViewChange={setUtilityView}
      />
      <div className="utility-content">
        {utility.activeView === 'diagnostics' ? (
          <DiagnosticsView useStore={useStore} />
        ) : (
          <SongDetailsView useStore={useStore} />
        )}
      </div>
    </section>
  );
}
