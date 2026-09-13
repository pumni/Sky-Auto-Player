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
const mockParams = new URLSearchParams(window.location.search);
const mockStartDelay = Number(mockParams.get('mockStartDelayMs'));
const mockStartResponseDelay = Number(mockParams.get('mockStartResponseDelayMs'));
const mockStatusQueryFailures = Number(mockParams.get('mockStatusQueryFailures'));
const mockStartFailureCode = mockParams.get('mockStartFailure');
const mockStartFailures: Record<string, { code: string; message: string }> = {
  target_not_found: {
    code: 'target_not_found',
    message:
      'Sky window was not found. Open Sky and make sure its window is visible, then try again.',
  },
  target_integrity_mismatch: {
    code: 'target_integrity_mismatch',
    message:
      'Sky is running with different permissions. Restart both applications at the same level.',
  },
  target_focus_failed: {
    code: 'target_focus_failed',
    message:
      'The validated Sky window could not be focused. Bring Sky to the foreground and try again.',
  },
};
const bridge = isTauri
  ? createTauriBridge()
  : createMockBridge({
      ...(Number.isFinite(mockDuration) && mockDuration > 0
        ? { playbackDurationMs: Math.min(mockDuration, 120_000) }
        : {}),
      ...(Number.isFinite(mockStartDelay) && mockStartDelay > 0
        ? { startDelayMs: Math.min(mockStartDelay, 15_000) }
        : {}),
      ...(Number.isFinite(mockStartResponseDelay) && mockStartResponseDelay > 0
        ? { startResponseDelayMs: Math.min(mockStartResponseDelay, 60_000) }
        : {}),
      ...(Number.isFinite(mockStatusQueryFailures) && mockStatusQueryFailures > 0
        ? { statusQueryFailures: Math.min(mockStatusQueryFailures, 10) }
        : {}),
      ...(mockParams.get('mockNeverCreateSession') === '1' ? { neverCreateSession: true } : {}),
      ...(mockParams.get('mockDropPlaybackSnapshots') === '1' ? { emitSnapshots: false } : {}),
      ...(mockParams.get('mockDropPlaybackStartConfirmation') === '1'
        ? { dropPlaybackStartConfirmation: true }
        : {}),
      ...(mockStartFailureCode && mockStartFailures[mockStartFailureCode]
        ? { startFailure: mockStartFailures[mockStartFailureCode] }
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
