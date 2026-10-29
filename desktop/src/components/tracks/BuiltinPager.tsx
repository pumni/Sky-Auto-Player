import { Check, ChevronLeft, ChevronRight } from 'lucide-react';
import {
  useEffect,
  useId,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LIBRARY_PAGE_SIZE } from '../../state/store';

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
    if (!editing) return;
    const focusTimer = window.setTimeout(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    }, 0);
    return () => window.clearTimeout(focusTimer);
  }, [editing]);

  const pageDigits = String(pageCount).length;
  const indicatorWidth = `calc(${pageDigits * 2 + 3}ch + 8px)`;
  const pagerWidth = `calc(${pageDigits * 2 + 3}ch + 88px)`;

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
      className="builtin-pager"
      aria-label="Built-in Songs pages"
      aria-busy={busy}
      data-editing={editing ? 'true' : 'false'}
      style={{ width: pagerWidth }}
      onKeyDown={onPagerKeyDown}
    >
      {editing ? (
        <div className="builtin-pager-page-slot" style={{ width: '100%' }} data-editing="true">
          <div className="builtin-pager-editor" ref={editorRef} onBlur={onEditorBlur}>
            <label className="visually-hidden" htmlFor={pageInputId}>
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
            <span className="builtin-pager-page-count" aria-hidden="true">
              / {pageCount}
            </span>
            <button
              className="builtin-pager-confirm"
              type="button"
              aria-label="Confirm page change"
              title="Confirm page change"
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => void commitEditing()}
            >
              <Check size={14} aria-hidden="true" />
            </button>
            <span id={pageInstructionsId} className="visually-hidden">
              Enter a whole page number from 1 to {pageCount}. Press Enter to navigate or Escape to
              cancel.
            </span>
            {pageError && (
              <span id={pageErrorId} className="builtin-pager-error" role="alert">
                {pageError}
              </span>
            )}
          </div>
        </div>
      ) : (
        <>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Previous page"
            title="Previous page"
            disabled={page === 1}
            onClick={() => void navigate(page - 1)}
          >
            <ChevronLeft size={16} aria-hidden="true" />
          </button>
          <div className="builtin-pager-page-slot" style={{ width: indicatorWidth }}>
            <button
              ref={triggerRef}
              className="builtin-pager-jump-trigger"
              type="button"
              title="Jump to page"
              aria-label={`${page} / ${pageCount}, Page ${page} of ${pageCount}. Jump to page`}
              aria-current="page"
              onClick={beginEditing}
            >
              {page} / {pageCount}
            </button>
          </div>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Next page"
            title="Next page"
            disabled={page === pageCount}
            onClick={() => void navigate(page + 1)}
          >
            <ChevronRight size={16} aria-hidden="true" />
          </button>
        </>
      )}
    </nav>
  );
}

export function BuiltinPager({ useStore }: { useStore: DesktopStoreHook }) {
  const total = useStore((store: DesktopStore) => store.library.resultTotal);
  const page = useStore((store: DesktopStore) => store.library.builtinPage.committedPage);
  const pendingPage = useStore((store: DesktopStore) => store.library.builtinPage.pendingPage);
  const failedPage = useStore((store: DesktopStore) => store.library.builtinPage.failedPage);
  const navigate = useStore((store: DesktopStore) => store.navigateBuiltinPage);
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
    <div className="builtin-pager-wrap">
      <span className="builtin-pager-range" aria-hidden="true">
        {rangeText}
      </span>
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
      <p className="builtin-pager-status" role="status" aria-live="polite" aria-atomic="true">
        {status}
      </p>
    </div>
  );
}
