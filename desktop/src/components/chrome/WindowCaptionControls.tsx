import { useCallback, useEffect, useRef, useState } from 'react';
import type { WindowControls } from '../../platform/windowControls';

interface WindowCaptionControlsProps {
  controls: WindowControls;
}

export function WindowCaptionControls({ controls }: WindowCaptionControlsProps) {
  const [maximized, setMaximized] = useState(false);
  const [windowActive, setWindowActive] = useState(() => document.hasFocus());
  const disposedRef = useRef(false);

  const refreshMaximized = useCallback(() => {
    void controls.isMaximized().then((value) => {
      if (!disposedRef.current) setMaximized(value);
    });
  }, [controls]);

  const refreshSettled = useCallback(() => {
    refreshMaximized();

    const requestFrame = window.requestAnimationFrame;
    if (typeof requestFrame !== 'function') {
      window.setTimeout(refreshMaximized, 50);
      return;
    }

    requestFrame(() => {
      if (disposedRef.current) return;
      requestFrame(() => {
        if (!disposedRef.current) refreshMaximized();
      });
    });
  }, [refreshMaximized]);

  useEffect(() => {
    disposedRef.current = false;
    let unsubscribe: (() => void) | undefined;

    refreshMaximized();
    void controls.onResize(refreshSettled).then((cleanup) => {
      if (disposedRef.current) cleanup();
      else unsubscribe = cleanup;
    });

    return () => {
      disposedRef.current = true;
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
        className="caption-button"
        type="button"
        aria-label="Minimize window"
        title="Minimize window"
        data-tauri-drag-region="false"
        onClick={() => void controls.minimize()}
      >
        <span className="caption-glyph caption-glyph-minimize" aria-hidden="true" />
      </button>
      <button
        className="caption-button"
        type="button"
        aria-label={maximizeLabel}
        title={maximizeLabel}
        data-tauri-drag-region="false"
        onClick={() => void controls.toggleMaximize().finally(refreshSettled)}
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
        onClick={() => void controls.close()}
      >
        <span className="caption-glyph caption-glyph-close" aria-hidden="true" />
      </button>
    </div>
  );
}
