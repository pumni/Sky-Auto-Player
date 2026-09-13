import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createMockBridge, createTauriBridge } from './bridge';
import { App } from './app/App';
import { ErrorBoundary } from './app/ErrorBoundary';
import './styles/tokens.css';
import './styles/reset.css';
import './styles/base.css';
import './styles/chrome.css';
import './styles/layout.css';
import './styles/workbench.css';
import './styles/tracks.css';
import './styles/player.css';
import './styles/overlays.css';
import './styles/themes.css';

// Tauri's bundled API uses the internal bridge, while the Windows WebView2
// page can expose the Tauri origin before that property is observable to the
// entry module. Keep browser/Playwright runs on the mock bridge, but recognize
// both stable Tauri origins for the packaged shell.
const isTauri =
  '__TAURI_INTERNALS__' in window ||
  'isTauri' in window ||
  window.location.protocol === 'tauri:' ||
  window.location.hostname === 'tauri.localhost';
const mockDuration = Number(
  new URLSearchParams(window.location.search).get('mockPlaybackDurationMs'),
);
const mockStartDelay = Number(new URLSearchParams(window.location.search).get('mockStartDelayMs'));
const bridge = isTauri
  ? createTauriBridge()
  : createMockBridge({
      ...(Number.isFinite(mockDuration) && mockDuration > 0
        ? { playbackDurationMs: Math.min(mockDuration, 120_000) }
        : {}),
      ...(Number.isFinite(mockStartDelay) && mockStartDelay > 0
        ? { startDelayMs: Math.min(mockStartDelay, 15_000) }
        : {}),
    });
const root = document.getElementById('root');

if (!root) throw new Error('desktop root element is missing');

createRoot(root).render(
  <StrictMode>
    <ErrorBoundary>
      <App bridge={bridge} />
    </ErrorBoundary>
  </StrictMode>,
);
