import { ChevronDown, ChevronLeft, ChevronRight } from 'lucide-react';
import { Button, Dialog, DialogTrigger, Popover } from 'react-aria-components';
import { useId, useRef, useState, type KeyboardEvent, type SubmitEvent } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LIBRARY_PAGE_SIZE } from '../../state/store';

export function BuiltinPager({ useStore }: { useStore: DesktopStoreHook }) {
  const total = useStore((store: DesktopStore) => store.library.resultTotal);
  const page = useStore((store: DesktopStore) => store.library.builtinPage.committedPage);
  const pendingPage = useStore((store: DesktopStore) => store.library.builtinPage.pendingPage);
  const failedPage = useStore((store: DesktopStore) => store.library.builtinPage.failedPage);
  const navigate = useStore((store: DesktopStore) => store.navigateBuiltinPage);
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
  const rangeText = `${firstRow}–${lastRow} of ${total} songs`;
  const rangeStatus = `Showing ${firstRow}–${lastRow} of ${total} songs.`;
  const status =
    pendingPage !== null
      ? `${rangeStatus} Loading page ${pendingPage} of ${pageCount}.`
      : failedPage !== null
        ? `${rangeStatus} Page ${failedPage} failed to load. Retry is available.`
        : pageCount === 1
          ? rangeStatus
          : `${rangeStatus} Page ${page} of ${pageCount}.`;
  const pageDigits = String(pageCount).length;
  const indicatorWidth = `calc(${pageDigits * 2 + 3}ch + 28px)`;

  const setPopoverOpen = (isOpen: boolean) => {
    setJumpOpen(isOpen);
    if (isOpen) {
      setJumpValue('');
      setJumpError(null);
      window.setTimeout(() => inputRef.current?.focus(), 0);
    } else {
      setJumpValue('');
      setJumpError(null);
    }
  };

  const closeJump = () => {
    setPopoverOpen(false);
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
    setPopoverOpen(false);
    await navigate(target);
    window.setTimeout(() => jumpTriggerRef.current?.focus(), 0);
  };

  const onJumpKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    closeJump();
  };

  const onPagerKeyDown = (event: KeyboardEvent<HTMLElement>) => {
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
    <div className="builtin-pager-wrap">
      <span className="builtin-pager-range" aria-hidden="true">
        {rangeText}
      </span>
      {pageCount > 1 && (
        <nav
          className="builtin-pager"
          aria-label="Built-in Songs pages"
          aria-busy={pendingPage !== null}
          onKeyDown={onPagerKeyDown}
        >
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Previous page"
            title="Previous page"
            disabled={page === 1}
            onClick={() => void navigate(page - 1)}
          >
            <ChevronLeft size={14} aria-hidden="true" />
            <span>Previous</span>
          </button>
          <DialogTrigger isOpen={jumpOpen} onOpenChange={setPopoverOpen}>
            <Button
              ref={jumpTriggerRef}
              className="builtin-pager-jump-trigger"
              type="button"
              style={{ width: indicatorWidth }}
              aria-label={`${page} / ${pageCount}, Page ${page} of ${pageCount}. Go to page`}
              aria-current="page"
              aria-expanded={jumpOpen}
              aria-controls={jumpOpen ? jumpPopoverId : undefined}
            >
              <span>
                {page} / {pageCount}
              </span>
              <ChevronDown size={12} aria-hidden="true" />
            </Button>
            <Popover
              id={jumpPopoverId}
              className="builtin-pager-popover"
              placement="top end"
              offset={6}
            >
              <Dialog aria-label="Go to page" className="builtin-pager-dialog">
                <form className="builtin-pager-jump" noValidate onSubmit={submitJump}>
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
                  {jumpError && (
                    <p id={jumpErrorId} className="builtin-pager-error" role="alert">
                      {jumpError}
                    </p>
                  )}
                </form>
              </Dialog>
            </Popover>
          </DialogTrigger>
          <button
            className="builtin-pager-control"
            type="button"
            aria-label="Next page"
            title="Next page"
            disabled={page === pageCount}
            onClick={() => void navigate(page + 1)}
          >
            <span>Next</span>
            <ChevronRight size={14} aria-hidden="true" />
          </button>
        </nav>
      )}
      <p className="builtin-pager-status" role="status" aria-live="polite" aria-atomic="true">
        {status}
      </p>
    </div>
  );
}
