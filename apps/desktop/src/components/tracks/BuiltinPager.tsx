import { Check, ChevronLeft, ChevronRight, LocateFixed } from 'lucide-react';
import {
  type FocusEvent,
  type KeyboardEvent,
  type RefObject,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LIBRARY_PAGE_SIZE } from '../../state/store';
import { Button } from '../ui/button';

interface BuiltinPagerNavigationProps {
  page: number;
  pageCount: number;
  busy: boolean;
  navigate: (page: number) => Promise<void>;
  triggerRef: RefObject<HTMLButtonElement | null>;
  editorFocusIntentRef: RefObject<boolean>;
}

function BuiltinPagerNavigation({
  page,
  pageCount,
  busy,
  navigate,
  triggerRef,
  editorFocusIntentRef,
}: BuiltinPagerNavigationProps) {
  const [editing, setEditing] = useState(false);
  const [pageValue, setPageValue] = useState('');
  const [pageError, setPageError] = useState<string | null>(null);
  const editorRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const pageInputId = useId();
  const pageInstructionsId = useId();
  const pageErrorId = useId();

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  const pageDigits = String(pageCount).length;
  // Reserve tabular digits plus the spaced slash. The zero glyph's ch width is
  // wider than the actual page label in several host fonts, which centered an
  // over-wide control and broke its visual spacing on Linux.
  const indicatorWidth = `max(36px, calc(${pageDigits * 1.1 + 0.6}em + 12px))`;
  const pagerWidth = `calc(${indicatorWidth} + 72px)`;

  const focusTrigger = () => {
    window.setTimeout(() => triggerRef.current?.focus(), 0);
  };

  const focusTriggerIfUnclaimed = (editingInput: HTMLInputElement | null) => {
    window.setTimeout(() => {
      if (document.activeElement !== document.body && document.activeElement !== editingInput) {
        return;
      }
      triggerRef.current?.focus();
    }, 0);
  };

  const beginEditing = () => {
    setPageValue(String(page));
    setPageError(null);
    setEditing(true);
  };

  const cancelEditing = (restoreFocus = true) => {
    setEditing(false);
    setPageValue('');
    setPageError(null);
    if (restoreFocus) editorFocusIntentRef.current = false;
    if (restoreFocus) focusTrigger();
  };

  const commitEditing = async () => {
    if (!/^[1-9]\d*$/.test(pageValue)) {
      setPageError(`Enter a whole page number from 1 to ${pageCount}.`);
      return;
    }
    const target = Number(pageValue);
    if (!Number.isSafeInteger(target) || target > pageCount) {
      setPageError(`Enter a whole page number from 1 to ${pageCount}.`);
      return;
    }
    const editingInput = inputRef.current;
    editorFocusIntentRef.current = false;
    setEditing(false);
    setPageValue('');
    setPageError(null);
    await navigate(target);
    focusTriggerIfUnclaimed(editingInput);
  };

  const onEditorKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      cancelEditing();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      void commitEditing();
    }
  };

  const onEditorBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (event.relatedTarget instanceof Node && editorRef.current?.contains(event.relatedTarget)) {
      return;
    }
    if (event.relatedTarget instanceof Node) editorFocusIntentRef.current = false;
    cancelEditing(false);
  };

  const onPagerKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (editing) return;
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    if (!(event.target instanceof HTMLButtonElement)) return;
    const controls = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button'));
    const currentIndex = controls.indexOf(event.target);
    if (currentIndex < 0) return;
    const direction = event.key === 'ArrowRight' ? 1 : -1;
    let nextIndex = currentIndex + direction;
    while (nextIndex >= 0 && nextIndex < controls.length && controls[nextIndex]?.disabled) {
      nextIndex += direction;
    }
    const nextControl = controls[nextIndex];
    if (!nextControl) return;
    event.preventDefault();
    nextControl.focus();
  };

  return (
    <nav
      className="grid h-8 min-w-0 flex-none grid-cols-[32px_minmax(0,1fr)_32px] items-center justify-self-end gap-1 text-[11px] tabular-nums whitespace-nowrap data-[editing=true]:grid-cols-1"
      aria-label="Built-in Songs pages"
      data-slot="builtin-pager-navigation"
      aria-busy={busy}
      data-editing={editing ? 'true' : 'false'}
      style={{ width: pagerWidth }}
      onKeyDown={onPagerKeyDown}
    >
      {editing ? (
        <div
          className="relative grid h-8 min-w-9 place-items-center"
          style={{ width: '100%' }}
          data-editing="true"
        >
          <div
            className="relative grid h-8 w-full grid-cols-[minmax(0,1fr)_auto_32px] items-center gap-[3px]"
            ref={editorRef}
            onBlur={onEditorBlur}
          >
            <label className="sr-only" htmlFor={pageInputId}>
              Page number
            </label>
            <input
              ref={inputRef}
              id={pageInputId}
              type="text"
              inputMode="numeric"
              autoComplete="off"
              size={pageDigits}
              value={pageValue}
              aria-label={`Page number, from 1 to ${pageCount}`}
              aria-invalid={pageError !== null}
              aria-describedby={
                pageError ? `${pageInstructionsId} ${pageErrorId}` : pageInstructionsId
              }
              onFocus={() => {
                editorFocusIntentRef.current = true;
              }}
              onChange={(event) => {
                setPageValue(event.currentTarget.value);
                setPageError(null);
              }}
              onKeyDown={onEditorKeyDown}
            />
            <span
              data-slot="builtin-pager-page-count"
              className="whitespace-nowrap text-[11px] tabular-nums text-text-tertiary"
              aria-hidden="true"
            >
              / {pageCount}
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-8 p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
              type="button"
              aria-label="Confirm page change"
              data-slot="builtin-pager-confirm"
              title="Confirm page change"
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => void commitEditing()}
            >
              <Check size={14} aria-hidden="true" />
            </Button>
            <span id={pageInstructionsId} className="sr-only">
              Enter a whole page number from 1 to {pageCount}. Press Enter to navigate or Escape to
              cancel.
            </span>
            {pageError && (
              <span
                id={pageErrorId}
                className="absolute bottom-[calc(100%+5px)] left-1/2 z-10 max-w-[calc(100vw-24px)] -translate-x-1/2 whitespace-nowrap rounded-control border border-border bg-popover px-2 py-1 text-[11px] leading-[1.4] text-danger shadow-lg"
                role="alert"
              >
                {pageError}
              </span>
            )}
          </div>
        </div>
      ) : (
        <>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 p-0 text-[11px] tabular-nums text-text-secondary hover:bg-control-hover hover:text-foreground disabled:hover:!bg-transparent"
            type="button"
            aria-label="Previous page"
            title="Previous page"
            disabled={page === 1}
            onClick={() => void navigate(page - 1)}
          >
            <ChevronLeft size={16} aria-hidden="true" />
          </Button>
          <div
            className="relative grid h-8 min-w-9 place-items-center"
            style={{ width: indicatorWidth }}
          >
            <Button
              variant="ghost"
              size="sm"
              ref={triggerRef}
              className="h-8 w-full min-w-0 bg-transparent px-1 py-1 text-[11px] tabular-nums text-foreground hover:bg-control-hover"
              data-page-jump="true"
              data-slot="builtin-pager-jump-trigger"
              type="button"
              title="Jump to page"
              aria-label={`${page} / ${pageCount}, Page ${page} of ${pageCount}. Jump to page`}
              aria-current="page"
              onClick={beginEditing}
            >
              {page} / {pageCount}
            </Button>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 p-0 text-[11px] tabular-nums text-text-secondary hover:bg-control-hover hover:text-foreground disabled:hover:!bg-transparent"
            type="button"
            aria-label="Next page"
            title="Next page"
            disabled={page === pageCount}
            onClick={() => void navigate(page + 1)}
          >
            <ChevronRight size={16} aria-hidden="true" />
          </Button>
        </>
      )}
    </nav>
  );
}

interface BuiltinPagerProps {
  useStore: DesktopStoreHook;
  onLocateSelected?: () => void;
}

export function BuiltinPager({ useStore, onLocateSelected }: BuiltinPagerProps) {
  const total = useStore((store: DesktopStore) => store.library.resultTotal);
  const page = useStore((store: DesktopStore) => store.library.builtinPage.committedPage);
  const pendingPage = useStore((store: DesktopStore) => store.library.builtinPage.pendingPage);
  const failedPage = useStore((store: DesktopStore) => store.library.builtinPage.failedPage);
  const navigate = useStore((store: DesktopStore) => store.navigateBuiltinPage);
  const canLocateSelected = useStore((store: DesktopStore) => {
    const library = store.library;
    return (
      onLocateSelected !== undefined &&
      library.source.kind === 'smart' &&
      library.source.id === 'builtin' &&
      !library.loading &&
      library.selectedSongId !== null &&
      library.selectedSongIndex !== null &&
      library.selectedSongIndex >= 0 &&
      library.selectedSongIndex < library.resultTotal &&
      library.membershipRevision === library.indexMembershipRevision
    );
  });
  const pageCount = Math.ceil(total / LIBRARY_PAGE_SIZE);
  const previousPageCountRef = useRef(pageCount);
  const editorFocusIntentRef = useRef(false);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const pageCountChanged = previousPageCountRef.current !== pageCount;
    previousPageCountRef.current = pageCount;
    if (!pageCountChanged) return;
    const shouldRestoreFocus = editorFocusIntentRef.current;
    editorFocusIntentRef.current = false;
    if (shouldRestoreFocus) triggerRef.current?.focus();
  }, [pageCount]);

  if (total === 0) return null;

  const firstRow = (page - 1) * LIBRARY_PAGE_SIZE + 1;
  const lastRow = Math.min(page * LIBRARY_PAGE_SIZE, total);
  const rangeText = `${firstRow}–${lastRow} of ${total} songs`;
  const rangeStatus = `Showing ${firstRow}–${lastRow} of ${total} songs`;
  const status =
    pendingPage !== null
      ? `${rangeStatus}. Loading page ${pendingPage} of ${pageCount}.`
      : failedPage !== null
        ? `${rangeStatus}. Page ${failedPage} failed to load. Retry is available.`
        : pageCount === 1
          ? `${rangeStatus}.`
          : `${rangeStatus}. Page ${page} of ${pageCount}.`;

  return (
    <div
      data-slot="builtin-pager"
      className="relative grid min-w-0 flex-none grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-t border-border px-3 py-0.5 pl-[18px] text-text-secondary @max-[640px]/track-browser:gap-x-2 @max-[640px]/track-browser:px-1.5 @max-[640px]/track-browser:pl-[18px]"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span
          data-slot="builtin-pager-range"
          className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] tabular-nums text-text-tertiary"
          aria-hidden="true"
        >
          {rangeText}
        </span>
        {onLocateSelected && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 flex-none p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
            type="button"
            aria-label="Locate selected"
            data-slot="builtin-pager-locate"
            title="Go to selected song"
            disabled={!canLocateSelected}
            onClick={onLocateSelected}
          >
            <LocateFixed size={16} aria-hidden="true" />
          </Button>
        )}
      </div>
      {pageCount > 1 && (
        <BuiltinPagerNavigation
          key={pageCount}
          page={page}
          pageCount={pageCount}
          busy={pendingPage !== null}
          navigate={navigate}
          triggerRef={triggerRef}
          editorFocusIntentRef={editorFocusIntentRef}
        />
      )}
      <p
        className="sr-only"
        data-slot="builtin-pager-status"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {status}
      </p>
    </div>
  );
}
