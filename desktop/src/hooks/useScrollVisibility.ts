import { useEffect, useRef } from 'react';

const SCROLL_HIDE_DELAY_MS = 750;

export function useScrollVisibility<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    let hideTimer: number | undefined;
    const onScroll = () => {
      element.classList.add('is-scrolling');
      if (hideTimer !== undefined) window.clearTimeout(hideTimer);
      hideTimer = window.setTimeout(() => {
        element.classList.remove('is-scrolling');
        hideTimer = undefined;
      }, SCROLL_HIDE_DELAY_MS);
    };

    element.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      element.removeEventListener('scroll', onScroll);
      if (hideTimer !== undefined) window.clearTimeout(hideTimer);
      element.classList.remove('is-scrolling');
    };
  }, []);

  return ref;
}
