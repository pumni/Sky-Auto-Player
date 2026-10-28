import { useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
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
  const pageCount = Math.ceil(total / LIBRARY_PAGE_SIZE);

  if (total <= LIBRARY_PAGE_SIZE) return null;

  const firstRow = (page - 1) * LIBRARY_PAGE_SIZE + 1;
  const lastRow = Math.min(page * LIBRARY_PAGE_SIZE, total);
  const status =
    pendingPage === null
      ? `Showing ${firstRow}–${lastRow} of ${total} songs. Page ${page} of ${pageCount}.`
      : `Showing ${firstRow}–${lastRow} of ${total} songs. Loading page ${pendingPage} of ${pageCount}.`;

  const openJump = () => {
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

  const submitJump = (event: FormEvent<HTMLFormElement>) => {
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
    closeJump();
    void navigate(target);
  };

  const onJumpKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    closeJump();
  };

  const pageButtons = pageItems(pageCount, page).map((item, index) =>
    typeof item === 'number' ? (
      <button
        key={item}
        className="builtin-pager-page"
        type="button"
        aria-label={`Page ${item}`}
        aria-current={item === page ? 'page' : undefined}
        onClick={() => void navigate(item)}
      >
        {item}
      </button>
    ) : (
      <span key={`${item}-${index}`} className="builtin-pager-ellipsis" aria-hidden="true">
        …
      </span>
    ),
  );

  return (
    <div className="builtin-pager-wrap">
      <nav
        className="builtin-pager"
        aria-label="Built-in Songs pages"
        aria-busy={pendingPage !== null}
      >
        <div className="builtin-pager-wide">
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
        </div>
        <div className="builtin-pager-narrow">
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Previous page"
            disabled={page === 1}
            onClick={() => void navigate(page - 1)}
          >
            Previous
          </button>
          <span>
            Page {page} of {pageCount}
          </span>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Next page"
            disabled={page === pageCount}
            onClick={() => void navigate(page + 1)}
          >
            Next
          </button>
        </div>
        <button
          ref={jumpTriggerRef}
          className="builtin-pager-jump-trigger"
          type="button"
          onClick={openJump}
        >
          Go to page
        </button>
        {jumpOpen && (
          <form className="builtin-pager-jump" noValidate onSubmit={submitJump}>
            <label htmlFor="builtin-page-jump">Go to page</label>
            <input
              ref={inputRef}
              id="builtin-page-jump"
              type="text"
              inputMode="numeric"
              pattern="[1-9][0-9]*"
              value={jumpValue}
              aria-invalid={jumpError !== null}
              aria-describedby={jumpError ? 'builtin-page-jump-error' : undefined}
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
      <p className="builtin-pager-status" role="status" aria-live="polite" aria-atomic="true">
        {status}
      </p>
      {jumpError && (
        <p id="builtin-page-jump-error" className="builtin-pager-error" role="alert">
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
