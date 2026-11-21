import { useCallback, useEffect, useRef, useState } from 'react';
import type { WindowControls } from '../../platform/windowControls';

interface WindowCaptionControlsProps {
  controls: WindowControls;
}

export function WindowCaptionControls({ controls }: WindowCaptionControlsProps) {
  const [maximized, setMaximized] = useState(false);
  const [windowActive, setWindowActive] = useState(() => document.hasFocus());
  const mountedRef = useRef(true);
  const rafIdRef = useRef<number | null>(null);
  const timeoutIdRef = useRef<number | null>(null);

  const refreshMaximized = useCallback(() => {
    void controls
      .isMaximized()
      .then((value) => {
        if (mountedRef.current) setMaximized(value);
      })
      .catch(() => {});
  }, [controls]);

  const refreshSettled = useCallback(() => {
    const requestFrame = window.requestAnimationFrame;
    if (typeof requestFrame !== 'function') {
      if (timeoutIdRef.current !== null) window.clearTimeout(timeoutIdRef.current);
      timeoutIdRef.current = window.setTimeout(() => {
        timeoutIdRef.current = null;
        refreshMaximized();
      }, 50);
      return;
    }

    if (rafIdRef.current !== null) window.cancelAnimationFrame(rafIdRef.current);
    rafIdRef.current = requestFrame(() => {
      rafIdRef.current = requestFrame(() => {
        rafIdRef.current = null;
        if (mountedRef.current) refreshMaximized();
      });
    });
  }, [refreshMaximized]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (timeoutIdRef.current !== null) {
        window.clearTimeout(timeoutIdRef.current);
        timeoutIdRef.current = null;
      }
      if (rafIdRef.current !== null) {
        window.cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    void controls
      .isMaximized()
      .then((value) => {
        if (!disposed) setMaximized(value);
      })
      .catch(() => {});

    const handleResize = () => {
      refreshMaximized();
      refreshSettled();
    };

    void controls
      .onResize(handleResize)
      .then((cleanup) => {
        if (disposed) cleanup();
        else unsubscribe = cleanup;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [controls, refreshMaximized, refreshSettled]);

  useEffect(() => {
    const activate = () => setWindowActive(true);
    const deactivate = () => setWindowActive(false);
    window.addEventListener('focus', activate);
    window.addEventListener('blur', deactivate);
    return () => {
      window.removeEventListener('focus', activate);
      window.removeEventListener('blur', deactivate);
    };
  }, []);

  const maximizeLabel = maximized ? 'Restore window' : 'Maximize window';
  return (
    <div
      className={`window-caption-controls${windowActive ? '' : ' is-inactive'}`}
      aria-label="Window controls"
      data-tauri-drag-region="false"
    >
      <button
        className="caption-button caption-button-minimize"
        type="button"
        aria-label="Minimize window"
        title="Minimize window"
        data-tauri-drag-region="false"
        onClick={() => void controls.minimize().catch(() => {})}
      >
        <span className="caption-glyph caption-glyph-minimize" aria-hidden="true" />
      </button>
      <button
        className="caption-button caption-button-zoom"
        type="button"
        aria-label={maximizeLabel}
        title={maximizeLabel}
        data-tauri-drag-region="false"
        onClick={() =>
          void controls
            .toggleMaximize()
            .catch(() => {})
            .finally(refreshSettled)
        }
      >
        <span
          className={`caption-glyph ${maximized ? 'caption-glyph-restore' : 'caption-glyph-maximize'}`}
          aria-hidden="true"
        />
      </button>
      <button
        className="caption-button caption-button-close"
        type="button"
        aria-label="Close window"
        title="Close window"
        data-tauri-drag-region="false"
        onClick={() => void controls.close().catch(() => {})}
      >
        <span className="caption-glyph caption-glyph-close" aria-hidden="true" />
      </button>
    </div>
  );
}
