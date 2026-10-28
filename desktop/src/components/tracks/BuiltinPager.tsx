import { useId, useRef, useState, type KeyboardEvent, type SubmitEvent } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LIBRARY_PAGE_SIZE } from '../../state/store';

function pageItems(
  total: number,
  current: number,
): Array<number | 'ellipsis-left' | 'ellipsis-right'> {
  if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1);
  const pages = [...new Set([1, current - 1, current, current + 1, total])]
    .filter((page) => page >= 1 && page <= total)
    .sort((left, right) => left - right);
  const items: Array<number | 'ellipsis-left' | 'ellipsis-right'> = [];
  pages.forEach((page, index) => {
    if (index > 0 && page - pages[index - 1]! > 1) {
      items.push(index === 1 ? 'ellipsis-left' : 'ellipsis-right');
    }
    items.push(page);
  });
  return items;
}

export function BuiltinPager({ useStore }: { useStore: DesktopStoreHook }) {
  const total = useStore((store: DesktopStore) => store.library.resultTotal);
  const page = useStore((store: DesktopStore) => store.library.builtinPage.committedPage);
  const pendingPage = useStore((store: DesktopStore) => store.library.builtinPage.pendingPage);
  const failedPage = useStore((store: DesktopStore) => store.library.builtinPage.failedPage);
  const navigate = useStore((store: DesktopStore) => store.navigateBuiltinPage);
  const retry = useStore((store: DesktopStore) => store.retryBuiltinPage);
  const [jumpOpen, setJumpOpen] = useState(false);
  const [jumpValue, setJumpValue] = useState('');
  const [jumpError, setJumpError] = useState<string | null>(null);
  const jumpTriggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const jumpPopoverId = useId();
  const jumpInputId = useId();
  const jumpErrorId = useId();
  const pageCount = Math.ceil(total / LIBRARY_PAGE_SIZE);

  if (total === 0) return null;

  const firstRow = (page - 1) * LIBRARY_PAGE_SIZE + 1;
  const lastRow = Math.min(page * LIBRARY_PAGE_SIZE, total);
  const rangeStatus = `Showing ${firstRow}–${lastRow} of ${total} songs.`;
  const status =
    pendingPage === null
      ? pageCount === 1
        ? rangeStatus
        : `${rangeStatus} Page ${page} of ${pageCount}.`
      : `${rangeStatus} Loading page ${pendingPage} of ${pageCount}.`;

  const openJump = (trigger: HTMLButtonElement) => {
    jumpTriggerRef.current = trigger;
    setJumpOpen(true);
    setJumpValue('');
    setJumpError(null);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  };

  const closeJump = () => {
    setJumpOpen(false);
    setJumpValue('');
    setJumpError(null);
    window.setTimeout(() => jumpTriggerRef.current?.focus(), 0);
  };

  const submitJump = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!/^[1-9]\d*$/.test(jumpValue)) {
      setJumpError(`Enter a whole page number from 1 to ${pageCount}.`);
      return;
    }
    const target = Number(jumpValue);
    if (!Number.isSafeInteger(target) || target > pageCount) {
      setJumpError(`Enter a whole page number from 1 to ${pageCount}.`);
      return;
    }
    setJumpOpen(false);
    setJumpValue('');
    setJumpError(null);
    await navigate(target);
    window.setTimeout(() => jumpTriggerRef.current?.focus(), 0);
  };

  const onJumpKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    closeJump();
  };

  const pageButtons = pageItems(pageCount, page).map((item, index) => {
    if (typeof item !== 'number') {
      return (
        <span key={`${item}-${index}`} className="builtin-pager-ellipsis" aria-hidden="true">
          …
        </span>
      );
    }
    const isCurrent = item === page;
    return (
      <button
        key={item}
        ref={isCurrent ? jumpTriggerRef : undefined}
        className="builtin-pager-page"
        type="button"
        aria-label={
          isCurrent ? `${item} / ${pageCount}, Page ${item} of ${pageCount}` : `Page ${item}`
        }
        aria-current={isCurrent ? 'page' : undefined}
        aria-expanded={isCurrent ? jumpOpen : undefined}
        aria-controls={isCurrent && jumpOpen ? jumpPopoverId : undefined}
        onClick={isCurrent ? (event) => openJump(event.currentTarget) : () => void navigate(item)}
      >
        {isCurrent ? (
          <>
            <span className="builtin-pager-current-wide">
              {item} / {pageCount}
            </span>
            <span className="builtin-pager-current-narrow">
              Page {item} of {pageCount}
            </span>
          </>
        ) : (
          item
        )}
      </button>
    );
  });

  return (
    <div className="builtin-pager-wrap">
      {pageCount > 1 && (
        <nav
          className="builtin-pager"
          aria-label="Built-in Songs pages"
          aria-busy={pendingPage !== null}
        >
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="First page"
            disabled={page === 1}
            onClick={() => void navigate(1)}
          >
            First
          </button>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Previous page"
            disabled={page === 1}
            onClick={() => void navigate(page - 1)}
          >
            Previous
          </button>
          {pageButtons}
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Next page"
            disabled={page === pageCount}
            onClick={() => void navigate(page + 1)}
          >
            Next
          </button>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Last page"
            disabled={page === pageCount}
            onClick={() => void navigate(pageCount)}
          >
            Last
          </button>
          {jumpOpen && (
            <form
              id={jumpPopoverId}
              className="builtin-pager-jump"
              noValidate
              onSubmit={submitJump}
            >
              <label htmlFor={jumpInputId}>Go to page</label>
              <input
                ref={inputRef}
                id={jumpInputId}
                type="text"
                inputMode="numeric"
                pattern="[1-9][0-9]*"
                value={jumpValue}
                aria-invalid={jumpError !== null}
                aria-describedby={jumpError ? jumpErrorId : undefined}
                onChange={(event) => {
                  setJumpValue(event.currentTarget.value);
                  setJumpError(null);
                }}
                onKeyDown={onJumpKeyDown}
              />
              <button type="submit">Go</button>
              <button type="button" onClick={closeJump}>
                Cancel
              </button>
            </form>
          )}
        </nav>
      )}
      <p className="builtin-pager-status" role="status" aria-live="polite" aria-atomic="true">
        {status}
      </p>
      {jumpError && (
        <p id={jumpErrorId} className="builtin-pager-error" role="alert">
          {jumpError}
        </p>
      )}
      {failedPage !== null && (
        <p className="builtin-pager-error" role="alert">
          Couldn’t load page {failedPage}.{' '}
          <button type="button" onClick={() => void retry()}>
            Retry page {failedPage}
          </button>
        </p>
      )}
    </div>
  );
}
