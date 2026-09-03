import { Copy, Minus, Square, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import type { WindowControls } from '../../platform/windowControls';

interface WindowCaptionControlsProps {
  controls: WindowControls;
}

export function WindowCaptionControls({ controls }: WindowCaptionControlsProps) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    const refresh = () => {
      void controls.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });
    };

    refresh();
    void controls.onResize(refresh).then((cleanup) => {
      if (disposed) cleanup();
      else unsubscribe = cleanup;
    });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [controls]);

  const maximizeLabel = maximized ? 'Restore window' : 'Maximize window';
  return (
    <div
      className="window-caption-controls"
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
        <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
      <button
        className="caption-button"
        type="button"
        aria-label={maximizeLabel}
        title={maximizeLabel}
        data-tauri-drag-region="false"
        onClick={() => void controls.toggleMaximize()}
      >
        {maximized ? (
          <Copy size={14} strokeWidth={1.5} aria-hidden="true" />
        ) : (
          <Square size={14} strokeWidth={1.5} aria-hidden="true" />
        )}
      </button>
      <button
        className="caption-button caption-button-close"
        type="button"
        aria-label="Close window"
        title="Close window"
        data-tauri-drag-region="false"
        onClick={() => void controls.close()}
      >
        <X size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
    </div>
  );
}
