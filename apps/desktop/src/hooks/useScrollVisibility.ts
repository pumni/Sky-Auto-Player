import { useCallback, useRef } from 'react';

const SCROLL_HIDE_DELAY_MS = 750;

export function useScrollVisibility<T extends HTMLElement>(active = true) {
  const detachRef = useRef<(() => void) | null>(null);

  return useCallback(
    (element: T | null) => {
      detachRef.current?.();
      detachRef.current = null;
      if (!active || !element) return;

      element.setAttribute('data-scroll-region', '');
      let hideTimer: number | undefined;
      const onScroll = () => {
        element.setAttribute('data-scrolling', 'true');
        if (hideTimer !== undefined) window.clearTimeout(hideTimer);
        hideTimer = window.setTimeout(() => {
          element.removeAttribute('data-scrolling');
          hideTimer = undefined;
        }, SCROLL_HIDE_DELAY_MS);
      };

      element.addEventListener('scroll', onScroll, { passive: true });
      detachRef.current = () => {
        element.removeEventListener('scroll', onScroll);
        if (hideTimer !== undefined) window.clearTimeout(hideTimer);
        element.removeAttribute('data-scrolling');
        element.removeAttribute('data-scroll-region');
      };
    },
    [active],
  );
}
